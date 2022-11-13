use futures::channel::mpsc;
use std::sync::{Arc, Mutex};

use crate::test_fixtures::{Error, Request, Response};
use futures::{stream, SinkExt, StreamExt};

struct Client {
    tx: mpsc::Sender<Request>,
    rx: mpsc::Receiver<Response>,
}

struct Handler {
    tx: mpsc::Sender<Response>,
    rx: mpsc::Receiver<Request>,
}

struct Data {
    keep_serving: bool,
}

struct Service {
    next_client_id: u8,
    handlers: Vec<Handler>,
    data: Arc<Mutex<Data>>,
}

impl Service {
    fn client(&mut self) -> Result<Client, Error> {
        if self.next_client_id == u8::MAX {
            return Err(Error("exceeded max clients".to_owned()));
        }
        let client_id = self.next_client_id;
        self.next_client_id += 1;
        let (ctx, srx) = mpsc::channel(32);
        let (stx, crx) = mpsc::channel(32);
        self.handlers.push(Handler { tx: stx, rx: srx });
        Ok(Client { tx: ctx, rx: crx })
    }

    async fn process_request(req: Request, handler: &mut Handler, data: &mut Data) {
        let res = match req {
            Request::A => Response::A,
            Request::B => Response::B,
        };
        match handler.tx.send(res).await {
            Ok(()) => {}
            Err(e) => log::error!("failed to send response: {e:?}"),
        }
    }
    
    async fn keep_serving(&mut self) -> bool {
        self.data.lock().unwrap().keep_serving
    }

    async fn serve(&mut self) -> Result<(), Error> {
        while self.keep_serving().await {
            let mut handlers = Vec::new();
            for handler in &mut self.handlers {
                let data_arc = self.data.clone();
                handlers.push(Box::pin(async move {
                    let mut data = data_arc.lock().unwrap();
                    match handler.rx.next().await {
                        Some(req) => Self::process_request(req, handler, &mut *data).await,
                        None => {}
                    }
                }))
            }
            futures::future::select_all(handlers).await;
        }
        Ok(())
    }
}
