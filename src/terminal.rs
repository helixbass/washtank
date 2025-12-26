use crossterm::event::EventStream;
use tokio::sync::mpsc::Sender;
use tokio_stream::StreamExt;

use crate::World;

pub fn listen_to_crossterm_events(sender: Sender<World>) {
    tokio::spawn(async move {
        let mut event_stream = EventStream::new();

        while let Some(Ok(event)) = event_stream.next().await {
            sender.send(World::Crossterm(event)).await.unwrap();
        }

        panic!("kill everything")
    });
}
