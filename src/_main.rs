use async_std::task;
use futures::channel::mpsc::{self, Receiver};
use futures::{SinkExt, StreamExt};
use std::sync::{Arc, Mutex};

struct Service {
    handlers: Vec<Receiver<bool>>,
    data: Arc<Mutex<bool>>,
}

impl Service {
    async fn handle_requests(&mut self) {
        let mut handlers = Vec::new();
        for rx in &mut self.handlers {
            let data_arc = self.data.clone();
            handlers.push(Box::pin(async move {
                if let Some(val) = rx.next().await {
                    *data_arc.lock().unwrap() = val
                }
            }));
        }
        futures::future::select_all(handlers).await;
    }
}

fn main() {
    let mut svc = Service {
        handlers: Vec::new(),
        data: Arc::new(Mutex::new(true)),
    };
    let (mut tx, rx) = mpsc::channel(8);
    svc.handlers.push(rx);
    let jh = task::spawn(async move { svc.handle_requests().await });
    task::block_on(async move {
        tx.send(false).await.unwrap();
        jh.await
    });
}