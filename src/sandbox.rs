use futures::channel::mpsc::{Receiver, Sender};
use futures::{stream, SinkExt, StreamExt};

struct Client {
    tx: Sender<String>,
    rx: Receiver<String>,
}

struct Handler {
    tx: Sender<String>,
    rx: Receiver<String>,
}

struct Service {
    handlers: Vec<Handler>,
}

impl Service {
    async fn serve(&mut self) {
        loop {
            let num_handlers = self.handlers.len();
            stream::iter(&mut self.handlers).for_each_concurrent(
                num_handlers,
                |handler: &mut Handler| async move {
                    if let Some(s) = handler.rx.next().await {
                        handler.tx.send(s).await;
                    }
                },
            ).await;
        }
    }
}
