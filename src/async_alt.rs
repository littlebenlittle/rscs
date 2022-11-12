use futures::channel::mpsc;

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

struct Service {
    next_client_id: u8,
    handlers: Vec<Handler>,
}

impl Service {
    fn new() -> Self {
        Self {
            next_client_id: 0,
            handlers: Vec::new(),
        }
    }

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

    async fn process_request(req: Request, handler: &mut Handler) {
        match handler
            .tx
            .send(match req {
                Request::A => Response::A,
                Request::B => Response::B,
            })
            .await
        {
            Ok(()) => {}
            Err(e) => log::error!("failed to send response: {e:?}"),
        }
    }

    fn keep_serving(&self) -> bool {
        true
    }

    async fn serve(&mut self) -> Result<(), Error> {
        while self.keep_serving() {
            stream::iter(&mut self.handlers)
                .for_each_concurrent(None, |handler| async move {
                    if let Some(req) = handler.rx.next().await {
                        Self::process_request(req, handler).await;
                    }
                })
                .await;
        }
        Ok(())
    }
}
