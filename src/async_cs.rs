use async_trait::async_trait;
use futures::channel::mpsc;
use futures::{stream, StreamExt};

#[async_trait]
pub trait Client<Req, Res> {
    type Error;
    fn new(tx: mpsc::Sender<Req>, rx: mpsc::Receiver<Res>) -> Self;
    async fn send(&mut self, req: Req) -> Result<(), Self::Error>;
    async fn recv(&mut self) -> Option<Res>;
}

#[async_trait]
pub trait Handler<Req: Send, Res, Svc: Service<Req, Res>>: Send {
    type Error: std::error::Error + Send;
    async fn send(&mut self, res: Res) -> Result<(), Self::Error>;
    async fn recv(&mut self) -> Option<Req>;
}

#[async_trait]
pub trait Service<Req: Send, Res>: Sized {
    type Client: Client<Req, Res>;
    type ClientId: Eq;
    type Error: From<<Self::Handler as Handler<Req, Res, Self>>::Error>;
    type Handler: Handler<Req, Res, Self>;

    async fn process_request(req: Req, handler: &mut Self::Handler);
    fn client(&mut self) -> Result<Self::Client, Self::Error>;
    fn handlers(&mut self) -> Vec<&mut Self::Handler>;

    async fn serve(&mut self) -> Result<(), Self::Error> {
        loop {
            stream::iter(self.handlers()).for_each_concurrent(
                None,
                |handler: &mut Self::Handler| async move {
                    if let Some(req) = handler.recv().await {
                        Self::process_request(req, handler).await
                    }
                },
            ).await;
        }
    }
}

#[cfg(test)]
mod test {
    use std::collections::BTreeMap;

    use super::*;
    use crate::test_fixtures::{Error, Request, Response};
    use async_std::task;
    use futures::{channel::oneshot, SinkExt, StreamExt};

    impl From<mpsc::SendError> for Error {
        fn from(e: mpsc::SendError) -> Self {
            Self(format!("{e:?}"))
        }
    }

    struct TestClient {
        tx: mpsc::Sender<Request>,
        rx: mpsc::Receiver<Response>,
    }

    #[async_trait]
    impl Client<Request, Response> for TestClient {
        type Error = Error;
        fn new(tx: mpsc::Sender<Request>, rx: mpsc::Receiver<Response>) -> Self {
            Self { tx, rx }
        }
        async fn send(&mut self, req: Request) -> Result<(), Self::Error> {
            Ok(self.tx.send(req).await?)
        }
        async fn recv(&mut self) -> Option<Response> {
            self.rx.next().await
        }
    }

    struct TestHandler {
        tx: mpsc::Sender<Response>,
        rx: mpsc::Receiver<Request>,
    }

    #[async_trait]
    impl Handler<Request, Response, TestService> for TestHandler {
        type Error = Error;
        async fn send(&mut self, res: Response) -> Result<(), Self::Error> {
            Ok(self.tx.send(res).await?)
        }
        async fn recv(&mut self) -> Option<Request> {
            self.rx.next().await
        }
    }

    struct TestService {
        next_client_id: u8,
        handlers: BTreeMap<u8, TestHandler>,
        keep_serving: bool,
    }

    impl TestService {
        fn new() -> Self {
            Self {
                next_client_id: 0,
                handlers: BTreeMap::new(),
                keep_serving: true,
            }
        }
    }

    #[async_trait]
    impl Service<Request, Response> for TestService {
        type Error = Error;
        type Client = TestClient;
        type ClientId = u8;
        type Handler = TestHandler;

        fn client(&mut self) -> Result<Self::Client, Self::Error> {
            if self.next_client_id == u8::MAX {
                return Err(Error("exceeded max clients".to_owned()));
            }
            let id = self.next_client_id;
            self.next_client_id += 1;
            let (ctx, srx) = mpsc::channel(32);
            let (stx, crx) = mpsc::channel(32);
            self.handlers.insert(id, TestHandler { tx: stx, rx: srx });
            Ok(Self::Client::new(ctx, crx))
        }

        fn handlers(&mut self) -> Vec<&mut Self::Handler> {
            self.handlers.values_mut().collect()
        }

        async fn process_request(req: Request, handler: &mut Self::Handler) {
            match handler
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
    }

    #[test]
    fn process_one_request() -> Result<(), Box<dyn std::error::Error>> {
        let mut svc = TestService::new();
        let mut cli = svc.client()?;
        let jh = task::spawn(async move { svc.serve().await });
        task::block_on(async move {
            cli.send(Request::A).await?;
            assert_eq!(cli.recv().await, Some(Response::A));
            jh.cancel().await;
            Result::<(), Error>::Ok(())
        })?;
        Ok(())
    }
}
