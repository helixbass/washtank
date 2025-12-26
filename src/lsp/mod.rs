use std::process::Stdio;

use tokio::{
    io::{BufReader, BufWriter},
    process::Command,
    sync::mpsc::{Receiver, Sender},
};

use crate::{jsonrpc, RpcMessage, World};

pub fn run_rust_analyzer(sender: Sender<World>, receiver: Receiver<LspOutgoingMessage>) {
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
        let writer = jsonrpc::Writer::new(BufWriter::new(stdin));
    });
}

pub enum LspOutgoingMessage {}

pub enum LspIncomingMessage {}

impl TryFrom<RpcMessage> for LspIncomingMessage {
    type Error = String;

    fn try_from(value: RpcMessage) -> Result<Self, Self::Error> {
        unimplemented!()
    }
}
