use std::process::Stdio;

use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    sync::mpsc::{Receiver, Sender},
};

use crate::{jsonrpc::Reader, World};

pub fn run_rust_analyzer(sender: Sender<World>, receiver: Receiver<LspOutgoingMessage>) {
    let mut command = Command::new("rust-analyzer");
    command.stdout(Stdio::piped());

    let mut child = command.spawn().unwrap();
    let stdout = child.stdout.take().unwrap();

    let mut reader = jsonrpc::Reader::new(BufReader::new(stdout).lines());

    tokio::spawn(async move {
        let status = child.wait().await.unwrap();

        panic!("rust-analyzer finished")
    });

    tokio::spawn(async move {
        // while let Some(line) = reader.next_line().await.unwrap() {
        while let Some(message) = reader.next_message().await.unwrap() {
            unimplemented!()
        }
    });
}

pub enum LspOutgoingMessage {}

pub enum LspIncomingMessage {}
