use async_trait::async_trait;
use futures::{
    channel::mpsc::{self, SendError},
    future,
    future::FusedFuture,
    stream,
    stream::FusedStream,
    Future, FutureExt, Sink, SinkExt, Stream, StreamExt,
};
use std::{
    collections::BTreeMap,
    marker::PhantomData,
    pin::Pin,
    sync::{Arc, Mutex},
};

pub type ClientId = u32;

pub trait ClientHandler<Req, Res>: Stream<Item = Req> + Sink<Req> + FusedStream + Unpin {}

pub trait ServiceHandler<Req, Res>: Send + 'static {
    fn handle_request(&mut self, id: &ClientId, req: Req) -> Option<Res>;
}

pub struct Service<Req, Res, Cli, Hdl>
where
    Hdl: ServiceHandler<Req, Res>,
    Cli: ClientHandler<Req, Res>,
{
    handler: Arc<Mutex<Hdl>>,
    keep_serving: Arc<Mutex<bool>>,
    clients: Arc<Mutex<BTreeMap<ClientId, Cli>>>,
    next_id: Arc<Mutex<ClientId>>,
    _phantom_data: PhantomData<(Req, Res)>,
}

impl<Req, Res, Cli, Hdl> Service<Req, Res, Cli, Hdl>
where
    Cli: ClientHandler<Req, Res>,
    Hdl: ServiceHandler<Req, Res>,
{
    pub fn new(handler: Hdl) -> Self {
        Self {
            handler: Arc::new(Mutex::new(handler)),
            keep_serving: Arc::new(Mutex::new(true)),
            clients: Arc::new(Mutex::new(BTreeMap::new())),
            next_id: Arc::new(Mutex::new(0)),
            _phantom_data: PhantomData,
        }
    }

    fn register_clients<'a, T, E, S>(
        &mut self,
        client_stream: S,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>
    where
        T: Into<Cli> + Send + 'a,
        E: std::error::Error + Send + 'a,
        S: FusedStream<Item = Result<T, E>> + Send + 'a,
    {
        // clone for each future
        let clients = self.clients.clone();
        let next_id = self.next_id.clone();
        let client_stream = Box::pin(client_stream);
        Box::pin(async move {
            client_stream
                .for_each(|into_client: Result<T, E>| {
                    // clone for each incoming client
                    let clients = clients.clone();
                    let next_id = next_id.clone();
                    Box::pin(async move {
                        let client = match into_client {
                            Ok(c) => c.into(),
                            Err(_) => return,
                        };
                        let mut clients = clients.lock().unwrap();
                        let mut next_id = *next_id.lock().unwrap();
                        if next_id == ClientId::MAX {
                            // TODO notify client sender
                        } else {
                            clients.insert(next_id, client);
                            next_id += 1;
                        }
                    })
                })
                .await
        })
    }

    pub fn shutdown(&mut self) -> Pin<Box<dyn Future<Output = ()>>> {
        let keep_serving = self.keep_serving.clone();
        Box::pin(async move {
            *keep_serving.lock().unwrap() = false;
        })
    }

    pub async fn serve<'a, T, E, S>(&mut self, client_stream: S)
    where
        T: Into<Cli> + Send + 'a,
        E: std::error::Error + Send + 'a,
        S: Stream<Item = Result<T, E>> + FusedStream + Unpin + 'a,
    {
        let handler = self.handler.clone();
        let keep_serving = self.keep_serving.clone();
        let clients = self.clients.clone();
        while *keep_serving.lock().unwrap() {
            let mut clients = clients.lock().unwrap();
            let req_stream = stream::select_all(
                clients
                    .iter_mut()
                    .map(|(id, client)| stream::repeat(id).zip(client).fuse()),
            );
            futures::select!(
                (id, req) = req_stream => handle_next_request(id, req, &self.clients, &self.handler).await,
                cli = clients_stream => handle_new_client(cli, &self.clients, &self.next_id).await
            );
        }
    }
}

async fn handle_new_client<Req, Res, Cli>(
    client: Cli,
    clients: &Arc<Mutex<BTreeMap<ClientId, Cli>>>,
    next_id: &Arc<Mutex<ClientId>>,
) where
    Cli: ClientHandler<Req, Res>,
{
    let mut clients = clients.lock().unwrap();
    let mut next_id = *next_id.lock().unwrap();
    if next_id == ClientId::MAX {
        // TODO notify client sender
    } else {
        clients.insert(next_id, client);
        next_id += 1;
    }
}

async fn handle_next_request<Req, Res, Cli, Hdl>(
    id: ClientId,
    req: Req,
    clients: &Arc<Mutex<BTreeMap<ClientId, Cli>>>,
    handler: &Arc<Mutex<Hdl>>,
) where
    Cli: ClientHandler<Req, Res>,
    Hdl: ServiceHandler<Req, Res>,
{
    let mut handler = handler.lock().unwrap();
    if let Some(res) = handler.handle_request(&id, req) {
        clients
            .lock()
            .unwrap()
            .get_mut(&id)
            .unwrap()
            .send_response(res);
    }
}
