use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;

use lsp_types::{InitializeParams, InitializeResult};
use oelung_lantern::mpsc::Sender;
use squalid::_d;
use tokio::{
    io::{BufReader, BufWriter},
    process::Command,
    sync::{mpsc::Receiver, RwLock},
};

use crate::{
    jsonrpc::{self, Id},
    Error, RequestMessage, ResponseMessage, RpcMessage,
};

pub fn run_rust_analyzer(
    sender: Box<dyn Sender<LspIncomingMessage>>,
    mut receiver: Receiver<LspOutgoingMessage>,
) {
    let mut command = Command::new("rust-analyzer");
    command.stdout(Stdio::piped());
    command.stdin(Stdio::piped());

    let mut child = command.spawn().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stdin = child.stdin.take().unwrap();

    let mut reader = jsonrpc::Reader::new(BufReader::new(stdout));

    tokio::spawn(async move {
        let _status = child.wait().await.unwrap();

        panic!("rust-analyzer finished")
    });

    let requests: Arc<RwLock<Requests>> = _d();

    tokio::spawn({
        let requests = requests.clone();
        async move {
            loop {
                sender.send(
                    LspIncomingMessage::from_rpc_message(
                        reader.read_message().await.unwrap(),
                        &requests,
                    )
                    .await
                    .unwrap(),
                );
            }
        }
    });

    tokio::spawn(async move {
        let mut writer = jsonrpc::Writer::new(BufWriter::new(stdin));
        let mut next_id = 1;

        while let Some(message) = receiver.recv().await {
            let id = next_id;
            next_id += 1;
            requests
                .write()
                .await
                .insert(id.into(), LspOutgoingMessageType::from(&message));
            writer
                .write_rpc_message(&message.into_rpc_message(id))
                .await
                .unwrap();
        }

        panic!("rust-analyzer sender finished")
    });
}

pub enum LspOutgoingMessage {
    Initialize(InitializeParams),
}

impl LspOutgoingMessage {
    pub fn into_rpc_message(self, id: i64) -> RpcMessage {
        match self {
            Self::Initialize(initialize) => {
                RpcMessage::Request(RequestMessage::with_params(id, "initialize", initialize))
            }
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LspOutgoingMessageType {
    Initialize,
}

impl<'a> From<&'a LspOutgoingMessage> for LspOutgoingMessageType {
    fn from(value: &'a LspOutgoingMessage) -> Self {
        match value {
            LspOutgoingMessage::Initialize(_) => Self::Initialize,
        }
    }
}

#[derive(Debug)]
pub enum LspIncomingMessage {
    InitializeResult(InitializeResult),
}

impl LspIncomingMessage {
    pub async fn from_rpc_message(
        rpc_message: RpcMessage,
        requests: &Arc<RwLock<Requests>>,
    ) -> Result<Self, Error> {
        Ok(match rpc_message {
            RpcMessage::Response(response) => match response {
                ResponseMessage::Error(response) => unimplemented!(),
                ResponseMessage::Success(response) => {
                    let request_type =
                        *requests.read().await.get(&response.id).ok_or_else(|| {
                            Error::Lsp("Got response for non-existent request".into())
                        })?;
                    match request_type {
                        LspOutgoingMessageType::Initialize => Self::InitializeResult(
                            serde_json::from_value(response.result.unwrap())
                                .map_err(|_| Error::Lsp("Couldn't parse response".into()))?,
                        ),
                    }
                }
            },
            _ => unimplemented!(),
        })
    }
}

pub type Requests = HashMap<Id, LspOutgoingMessageType>;
