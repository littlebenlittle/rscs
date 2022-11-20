use async_std::task;
use futures::{
    channel::mpsc::{self, Receiver, Sender},
    SinkExt, StreamExt,
};
use futures::{future, Future, FutureExt};
use std::sync::{Arc, Mutex};

macro_rules! ok_or_log_err {
    ($x: expr) => {
        match $x {
            Ok(a) => a,
            Err(e) => {
                log::error!("{e:?}");
                return;
            }
        }
    };
}

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

type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, PartialEq, Clone)]
enum Message {
    Noop,
    Echo(String),
}

struct Client {
    req_tx: Sender<Message>,
    res_rx: Receiver<Message>,
}

impl Client {
    async fn run(
        mut self,
        mut res_tx: Sender<Result<Message>>,
        mut req_rx: Receiver<Message>,
    ) -> Result<()> {
        let res_fut = async move {
            while let Some(res) = self.res_rx.next().await {
                log::debug!("client got response: {res:?}");
                ok_or_log_err!(res_tx.send(Ok(res)).await);
            }
        };
        let req_fut = async move {
            while let Some(req) = req_rx.next().await {
                log::debug!("client sending request: {req:?}");
                ok_or_log_err!(self.req_tx.send(req).await);
            }
        };
        future::join(res_fut, req_fut).await;
        Ok(())
    }

    async fn run_while<F, O, Fut>(self, cls: F) -> Result<O>
    where
        F: Fn(Sender<Message>, Receiver<Result<Message>>) -> Fut,
        Fut: Future<Output = O>,
    {
        let (res_tx, res_rx) = mpsc::channel(0);
        let (req_tx, req_rx) = mpsc::channel(0);
        let (r1, r2) = future::join(self.run(res_tx, req_rx), cls(req_tx, res_rx)).await;
        r1?;
        Ok(r2)
    }
}

struct Handler {
    res_tx: Sender<Message>,
    req_rx: Receiver<Message>,
}

struct Service {
    shutdown_tx: Arc<Mutex<Sender<()>>>,
    shutdown_rx: Receiver<()>,
    client_tx: Arc<Mutex<Sender<Handler>>>,
    client_rx: Receiver<Handler>,
}

impl Service {
    async fn new() -> Result<Self> {
        let (shutdown_tx, shutdown_rx) = mpsc::channel(0);
        let (client_tx, client_rx) = mpsc::channel(0);
        Ok(Self {
            shutdown_tx: Arc::new(Mutex::new(shutdown_tx)),
            shutdown_rx,
            client_tx: Arc::new(Mutex::new(client_tx)),
            client_rx,
        })
    }

    fn client(&mut self) -> impl Future<Output = Result<Client>> {
        let client_tx = self.client_tx.clone();
        async move {
            let mut client_tx = client_tx.lock().unwrap();
            let (req_tx, req_rx) = mpsc::channel(0);
            let (res_tx, res_rx) = mpsc::channel(0);
            let client = Client { req_tx, res_rx };
            client_tx.send(Handler { res_tx, req_rx }).await?;
            Ok(client)
        }
    }

    fn shutdown(&mut self) -> impl Future<Output = Result<()>> {
        let tx = self.shutdown_tx.clone();
        async move { Ok(tx.lock().unwrap().send(()).await?) }
    }

    async fn serve(mut self) {
        self.client_rx
            .take_until(self.shutdown_rx.next())
            .for_each_concurrent(
                None,
                |Handler {
                     mut res_tx,
                     mut req_rx,
                 }| async move {
                    while let Some(req) = req_rx.next().await {
                        log::debug!("server got request: {req:?}");
                        ok_or_log_err!(res_tx.send(req).await);
                    }
                },
            )
            .await
    }

    async fn serve_while<F, O>(mut self, fut: F) -> O
    where
        F: Future<Output = O>,
    {
        let shutdown = self.shutdown();
        let ((), result) = future::join(
            self.serve(),
            fut.then(|result| async move {
                shutdown.await.unwrap();
                result
            }),
        )
        .await;
        result
    }
}

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    task::block_on(async {
        env_logger::init();
        let mut service = Service::new().await.unwrap();
        let client = service.client();
        service
            .serve_while(async move {
                client.await?
                    .run_while(|mut tx, mut rx| async move {
                        for msg in [
                            Message::Noop,
                            Message::Echo("test".into()),
                            Message::Noop,
                            Message::Echo("somthing".into()),
                            Message::Echo("else".into()),
                            Message::Noop,
                        ] {
                            tx.send(msg.clone()).await?;
                            assert_eq!(rx.next().await, Some(Ok(msg)));
                        }
                        log::debug!("closing req channel");
                        tx.close_channel(); // TODO figure out how close in Client::run_while
                        Result::<()>::Ok(())
                    })
                    .await
            })
            .await??;
        Result::<()>::Ok(())
    })?;
    Ok(())
}
