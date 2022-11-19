use async_std::task;
use futures::Future;
use futures::{channel::mpsc, stream, SinkExt, StreamExt};
use std::sync::{Arc, Mutex};

#[derive(Debug, PartialEq)]
struct Error(String);
impl std::error::Error for Error {}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

macro_rules! impl_error {
    ($t: ty) => {
        impl From<$t> for Error {
            fn from(e: $t) -> Self {
                Self(e.to_string())
            }
        }
    };
}

impl_error!(std::io::Error);
impl_error!(mpsc::SendError);

enum Request {
    A,
}

#[derive(Debug, PartialEq)]
enum Response {
    A,
}

struct Client {
    req_tx: mpsc::Sender<Request>,
    res_rx: mpsc::Receiver<Response>,
}

impl Client {
    async fn send(&mut self, req: Request) -> Result<(), Error> {
        Ok(self.req_tx.send(req).await?)
    }

    async fn recv(&mut self) -> Option<Response> {
        self.res_rx.next().await
    }
}

struct Handler {
    res_tx: mpsc::Sender<Response>,
    req_rx: mpsc::Receiver<Request>,
}

struct Service {
    handlers: Vec<Handler>,
    shutdown_tx: Arc<Mutex<mpsc::Sender<()>>>,
    shutdown_rx: mpsc::Receiver<()>,
}

impl Service {
    async fn new() -> Result<Self, Error> {
        let (shutdown_tx, shutdown_rx) = mpsc::channel(0);
        Ok(Self {
            handlers: Vec::new(),
            shutdown_tx: Arc::new(Mutex::new(shutdown_tx)),
            shutdown_rx,
        })
    }

    async fn client(&mut self) -> Result<Client, Error> {
        let (req_tx, req_rx) = mpsc::channel(0);
        let (res_tx, res_rx) = mpsc::channel(0);
        let client = Client { req_tx, res_rx };
        self.handlers.push(Handler { res_tx, req_rx });
        Ok(client)
    }

    fn shutdown(&mut self) -> impl Future<Output = Result<(), mpsc::SendError>> {
        let tx = self.shutdown_tx.clone();
        async move { tx.lock().unwrap().send(()).await }
    }

    async fn serve(mut self) {
        stream::iter(self.handlers)
            .flat_map_unordered(None, |Handler { res_tx, req_rx }| {
                req_rx.zip(stream::repeat(res_tx))
            })
            .take_until(self.shutdown_rx.next())
            .for_each_concurrent(None, |(req, mut tx)| async move {
                match req {
                    Request::A => match tx.send(Response::A).await {
                        Ok(()) => {}
                        Err(e) => log::error!("{e:?}"),
                    },
                }
            })
            .await
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    task::block_on(async {
        env_logger::init();
        let mut service = Service::new().await.unwrap();
        let mut client = service.client().await?;
        let shutdown = service.shutdown();
        let client_fut = async move {
            client.send(Request::A).await.unwrap();
            assert_eq!(client.recv().await, Some(Response::A));
            shutdown.await.unwrap();
        };
        futures::future::join(client_fut, service.serve()).await;
        Result::<(), Error>::Ok(())
    })?;
    Ok(())
}
