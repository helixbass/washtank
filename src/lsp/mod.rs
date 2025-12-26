use tokio::sync::mpsc::{Receiver, Sender};

use crate::World;

pub async fn run_rust_analyzer(sender: Sender<World>, receiver: Receiver<LspOutgoingMessage>) {
    unimplemented!()
}

pub enum LspOutgoingMessage {}

pub enum LspIncomingMessage {}
