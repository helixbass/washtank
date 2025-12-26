use std::process::Stdio;

use lsp_types::InitializeParams;
use tokio::{
    io::{BufReader, BufWriter},
    process::Command,
    sync::mpsc::{Receiver, Sender},
};

use crate::{jsonrpc, RequestMessage, RpcMessage, World};

pub fn run_rust_analyzer(sender: Sender<World>, mut receiver: Receiver<LspOutgoingMessage>) {
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

    tokio::spawn(async move {
        loop {
            sender.send(World::Lsp(
                LspIncomingMessage::try_from(reader.read_message().await.unwrap()).unwrap(),
            ));
        }
    });

    tokio::spawn(async move {
        let mut writer = jsonrpc::Writer::new(BufWriter::new(stdin));
        let mut next_id = 1;

        while let Some(message) = receiver.recv().await {
            writer
                .write_rpc_message(&message.into_rpc_message({
                    let id = next_id;
                    next_id += 1;
                    id
                }))
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

pub enum LspIncomingMessage {}

impl TryFrom<RpcMessage> for LspIncomingMessage {
    type Error = String;

    fn try_from(value: RpcMessage) -> Result<Self, Self::Error> {
        unimplemented!()
    }
}
