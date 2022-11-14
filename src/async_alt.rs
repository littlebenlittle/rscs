use crate::test_fixtures::Error;
use async_trait::async_trait;
use futures::channel::mpsc::{self, Receiver, Sender};
use futures::{Future, Sink, SinkExt, Stream, StreamExt};
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

pub trait ClientId: Ord + Copy {}

impl<T: Ord + Copy> ClientId for T {}

pub trait ClientData<Req, Res, Id: ClientId> {
    fn clients_mut(&mut self) -> Vec<(Id, &mut Sender<Res>, &mut Receiver<Req>)>;
    fn num_clients(&self) -> usize;
    fn add_client(&mut self, tx: Sender<Res>, rx: Receiver<Req>) -> Result<(), Error>;
}

struct Client<Req, Res> {
    tx: Sender<Req>,
    rx: Receiver<Res>,
}

impl<Req, Res> Client<Req, Res> {
    fn new(tx: Sender<Req>, rx: Receiver<Res>) -> Self {
        Self
    }
    fn send(&mut self) -> &mut Sender<Req>;
    fn recv(&mut self) -> &mut Receiver<Res>;
}

pub trait ServiceData<Req, Res, C: ClientId> {
    fn process_request(&mut self, client_id: C, req: Req) -> Option<Res>;
}

pub struct Service<Req: Send, Res, C, Id, Cd, Sd>
where
    Id: ClientId,
    C: Client<Req, Res>,
    Cd: ClientData<Req, Res, Id>,
    Sd: ServiceData<Req, Res, Id>,
{
    client_data: Cd,
    data: Arc<Mutex<Sd>>,
    keep_serving: Arc<Mutex<bool>>,
    _phandom_data: PhantomData<(Req, Res, C, Cd, Id)>,
}

impl<Req: Send, Res, C, Id, Cd, Sd> Service<Req, Res, C, Id, Cd, Sd>
where
    Id: ClientId,
    C: Client<Req, Res>,
    Cd: ClientData<Req, Res, Id>,
    Sd: ServiceData<Req, Res, Id>,
{
    pub fn new(data: Sd, client_data: Cd) -> Self {
        Self {
            client_data,
            data: Arc::new(Mutex::new(data)),
            keep_serving: Arc::new(Mutex::new(true)),
            _phandom_data: PhantomData,
        }
    }

    fn client(&mut self) -> Result<C, Error> {
        let (ctx, srx) = mpsc::channel(32);
        let (stx, crx) = mpsc::channel(32);
        self.client_data.add_client(stx, srx)?;
        Ok(C::new(ctx, crx))
    }

    fn shutdown(&mut self) -> Pin<Box<dyn Future<Output = ()>>> {
        let keep_serving_arc = self.keep_serving.clone();
        Box::pin(async move {
            *keep_serving_arc.lock().unwrap() = false;
        })
    }

    async fn serve(&mut self) -> Result<(), Error> {
        while *self.keep_serving.lock().unwrap() {
            let mut handlers = Vec::with_capacity(self.client_data.num_clients());
            for (client_id, tx, rx) in self.client_data.clients_mut() {
                let data_arc = self.data.clone();
                handlers.push(Box::pin(async move {
                    if let Some(req) = rx.next().await {
                        let res = {
                            let mut data = data_arc.lock().unwrap();
                            data.process_request(client_id, req)
                        };
                        if let Some(res) = res {
                            match tx.send(res).await {
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
    use std::collections::BTreeMap;

    use super::*;
    use crate::test_fixtures::{Error, Request, Response};
    use async_std::task;

    struct Data {}

    struct DirectClient {
        tx: Sender<Request>,
        rx: Receiver<Response>,
    }

    impl Client<Request, Response> for DirectClient {
        fn new(tx: Sender<Request>, rx: Receiver<Response>) -> Self {
            Self { tx, rx }
        }
    }

    type ClientId = u8;

    struct ClientList {
        handlers: BTreeMap<ClientId, (Sender<Response>, Receiver<Request>)>,
        next_id: ClientId,
    }

    impl ClientData<Request, Response, ClientId> for ClientList {
        fn add_client(&mut self, tx: Sender<Response>, rx: Receiver<Request>) -> Result<(), Error> {
            if self.next_id == u8::MAX - 1 {
                return Err(Error("num clients exceeds u8::MAX".to_owned()));
            }
            self.handlers.insert(self.next_id, (tx, rx));
            self.next_id += 1;
            Ok(())
        }

        fn num_clients(&self) -> usize {
            self.handlers.len()
        }

        fn clients_mut(
            &mut self,
        ) -> Vec<(ClientId, &mut Sender<Response>, &mut Receiver<Request>)> {
            self.handlers
                .iter_mut()
                .map(|(id, (tx, rx))| (*id, tx, rx))
                .collect()
        }
    }

    impl ServiceData<Request, Response, ClientId> for Data {
        fn process_request(&mut self, _client_id: ClientId, req: Request) -> Option<Response> {
            match req {
                Request::A => Some(Response::A),
                Request::B => Some(Response::B),
                _ => None,
            }
        }
    }

    #[test]
    fn process_one_request() -> Result<(), Box<dyn std::error::Error>> {
        env_logger::init();
        let mut svc = Service::<Request, Response, DirectClient, ClientId, ClientList, Data>::new(
            Data {},
            ClientList {
                handlers: BTreeMap::new(),
                next_id: 0,
            },
        );
        let mut cli = svc.client()?;
        let shutdown = svc.shutdown();
        let jh = task::spawn(async move { svc.serve().await });
        task::block_on(async move {
            cli.send(Request::A).await?;
            assert_eq!(cli.recv().await, Some(Response::A));
            cli.send(Request::Shutdown).await?;
            shutdown.await;
            jh.await?;
            Result::<(), Error>::Ok(())
        })?;
        Ok(())
    }
}
