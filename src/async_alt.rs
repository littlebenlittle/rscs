use crate::test_fixtures::Error;
use futures::channel::mpsc;
use futures::{SinkExt, StreamExt};
use std::sync::{Arc, Mutex};

struct Client<Req, Res> {
    tx: mpsc::Sender<Req>,
    rx: mpsc::Receiver<Res>,
}

impl<Req, Res> Client<Req, Res> {
    pub async fn send(&mut self, req: Req) -> Result<(), mpsc::SendError> {
        self.tx.send(req).await
    }

    pub async fn recv(&mut self) -> Option<Res> {
        self.rx.next().await
    }
}

struct Handler<Req, Res> {
    tx: mpsc::Sender<Res>,
    rx: mpsc::Receiver<Req>,
}

trait ServiceData<Req, Res> {
    fn keep_serving(&self) -> bool;
    fn process_request(&mut self, req: Req) -> Option<Res>;
}

struct Service<Req, Res, D: ServiceData<Req, Res>> {
    handlers: Vec<Handler<Req, Res>>,
    data: Arc<Mutex<D>>,
}

impl<Req, Res, D> Service<Req, Res, D>
where
    D: ServiceData<Req, Res>,
{
    pub fn new(data: D) -> Self {
        Self {
            handlers: Vec::new(),
            data: Arc::new(Mutex::new(data)),
        }
    }
}

impl<Req, Res, D: ServiceData<Req, Res>> Service<Req, Res, D> {
    fn client(&mut self) -> Result<Client<Req, Res>, Error> {
        let (ctx, srx) = mpsc::channel(32);
        let (stx, crx) = mpsc::channel(32);
        self.handlers.push(Handler { tx: stx, rx: srx });
        Ok(Client { tx: ctx, rx: crx })
    }

    async fn serve(&mut self) -> Result<(), Error> {
        while self.data.lock().unwrap().keep_serving() {
            let mut handlers = Vec::new();
            for handler in &mut self.handlers {
                let data_arc = self.data.clone();
                handlers.push(Box::pin(async move {
                    if let Some(req) = handler.rx.next().await {
                        let res = {
                            let mut data = data_arc.lock().unwrap();
                            data.process_request(req)
                        };
                        if let Some(res) = res {
                            match handler.tx.send(res).await {
                                Ok(()) => {}
                                Err(e) => log::error!("failed to send response: {e:?}"),
                            }
                        }
                    }
                }))
            }
            futures::future::select_all(handlers).await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::test_fixtures::{Error, Request, Response};
    use async_std::task;

    struct Data {
        keep_serving: bool,
    }

    impl From<mpsc::SendError> for Error {
        fn from(e: mpsc::SendError) -> Self {
            Self(format!("{e:?}"))
        }
    }

    impl ServiceData<Request, Response> for Data {
        fn keep_serving(&self) -> bool {
            self.keep_serving
        }
        fn process_request(&mut self, req: Request) -> Option<Response> {
            match req {
                Request::A => Some(Response::A),
                Request::B => Some(Response::B),
                Request::Shutdown => {
                    self.keep_serving = false;
                    None
                }
            }
        }
    }

    #[test]
    fn process_one_request() -> Result<(), Box<dyn std::error::Error>> {
        env_logger::init();
        let mut svc = Service::new(Data { keep_serving: true });
        let mut cli = svc.client()?;
        let jh = task::spawn(async move { svc.serve().await });
        task::block_on(async move {
            cli.send(Request::A).await?;
            assert_eq!(cli.recv().await, Some(Response::A));
            cli.send(Request::Shutdown).await?;
            jh.await?;
            Result::<(), Error>::Ok(())
        })?;
        Ok(())
    }
}
