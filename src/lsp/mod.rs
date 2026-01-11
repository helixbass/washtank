use std::collections::HashMap;
use std::pin::Pin;
use std::process::Stdio;
use std::str::FromStr;
use std::sync::Arc;

use lsp_types::{
    Hover, HoverParams, InitializeParams, InitializeResult, Position, TextDocumentIdentifier,
    TextDocumentPositionParams, Uri, WorkDoneProgressParams,
};
use oelung_lantern::mpsc::Sender;
use squalid::_d;
use tokio::{
    io::{BufReader, BufWriter},
    process::Command,
    sync::{mpsc::Receiver, RwLock},
};

use crate::{
    jsonrpc::{self, Id},
    Editor, Error, PrintedLine, RequestMessage, ResponseMessage, RpcMessage,
};

impl Editor {
    pub fn current_file_lsp_text_document_identifier(&self) -> TextDocumentIdentifier {
        TextDocumentIdentifier {
            uri: Uri::from_str(&format!(
                "file://{}",
                self.current_file
                    .as_named()
                    .path
                    .as_path()
                    .to_str()
                    .unwrap()
            ))
            .unwrap(),
        }
    }

    pub fn current_file_cursor_lsp_position(&self) -> Position {
        let cursor_line_num = *match self.cursor_printed_line() {
            PrintedLine::Line(line_num) => line_num,
            PrintedLine::Fold(_) => panic!("expected not to be called with fold"),
        };
        Position {
            line: u32::try_from(cursor_line_num).unwrap(),
            character: u32::from(self.cursor_position.column),
        }
    }

    pub fn current_file_cursor_lsp_text_document_position_params(
        &self,
    ) -> TextDocumentPositionParams {
        TextDocumentPositionParams {
            text_document: self.current_file_lsp_text_document_identifier(),
            position: self.current_file_cursor_lsp_position(),
        }
    }

    pub fn send_lsp_hover_under_cursor(
        &self,
    ) -> Option<Pin<Box<dyn Future<Output = ()> + Send + 'static>>> {
        if matches!(self.cursor_printed_line(), PrintedLine::Fold(_)) {
            return None;
        }

        let hover_message = LspOutgoingMessage::Hover(HoverParams {
            text_document_position_params: self
                .current_file_cursor_lsp_text_document_position_params(),
            work_done_progress_params: WorkDoneProgressParams {
                work_done_token: None,
            },
        });

        Some(Box::pin({
            let rust_analyzer_sender = self.rust_analyzer_sender.clone();
            async move {
                rust_analyzer_sender.send(hover_message).await.unwrap();
            }
        }))
    }
}

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
                sender
                    .send(
                        LspIncomingMessage::from_rpc_message(
                            reader.read_message().await.unwrap(),
                            &requests,
                        )
                        .await
                        .unwrap(),
                    )
                    .await;
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
    Hover(HoverParams),
}

impl LspOutgoingMessage {
    pub fn into_rpc_message(self, id: i64) -> RpcMessage {
        match self {
            Self::Initialize(initialize) => {
                RpcMessage::Request(RequestMessage::with_params(id, "initialize", initialize))
            }
            Self::Hover(hover) => {
                RpcMessage::Request(RequestMessage::with_params(id, "hover", hover))
            }
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LspOutgoingMessageType {
    Initialize,
    Hover,
}

impl<'a> From<&'a LspOutgoingMessage> for LspOutgoingMessageType {
    fn from(value: &'a LspOutgoingMessage) -> Self {
        match value {
            LspOutgoingMessage::Initialize(_) => Self::Initialize,
            LspOutgoingMessage::Hover(_) => Self::Hover,
        }
    }
}

#[derive(Debug)]
pub enum LspIncomingMessage {
    InitializeResult(InitializeResult),
    Hover(Hover),
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
                        LspOutgoingMessageType::Hover => Self::Hover(
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
