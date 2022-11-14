use async_trait::async_trait;
use futures::{StreamExt, channel::mpsc};
use std::{
    collections::BTreeMap,
    marker::PhantomData,
    pin::Pin,
    sync::{Arc, Mutex},
};

use futures::Stream;

type ClientId = u32;

#[async_trait]
pub trait Client<Req, Res> {
    async fn next_request(&mut self) -> Req;
    async fn send_response(&mut self, res: Res);
}

pub trait ServiceHandler<Req, Res> {
    fn handle_request(&mut self, id: &ClientId, req: Req) -> Option<Res>;
}

pub struct Service<Req, Res, Cli, Hdl, Cs>
where
    Hdl: ServiceHandler<Req, Res>,
    Cli: Client<Req, Res>,
    Cs: Stream<Item = Cli>,
{
    handler: Arc<Mutex<Hdl>>,
    client_stream: Arc<Mutex<Pin<Box<Cs>>>>,
    shutdown_stream: Arc<Mutex<Pin<Box<mpsc::Receiver<()>>>>>,
    keep_serving: Arc<Mutex<bool>>,
    clients: Arc<Mutex<BTreeMap<ClientId, Cli>>>,
    next_id: Arc<Mutex<ClientId>>,
    _phantom_data: PhantomData<(Req, Res)>,
}

impl<Req, Res, Cli, Hdl, Cs> Service<Req, Res, Cli, Hdl, Cs>
where
    Cli: Client<Req, Res>,
    Hdl: ServiceHandler<Req, Res>,
    Cs: Stream<Item = Cli>,
{
    pub async fn serve(&mut self) {
        while *self.keep_serving.lock().unwrap() {
            let handle_next_request = {
                let clients = self.clients.clone();
                let handler = self.handler.clone();
                async move {
                    let mut clients = clients.lock().unwrap();
                    futures::future::select_all(clients.iter_mut().map(|(id, client)| {
                        let handler = handler.clone();
                        Box::pin(async move {
                            let req = client.next_request().await;
                            let res = {
                                let mut handler = handler.lock().unwrap();
                                handler.handle_request(id, req)
                            };
                            if let Some(res) = res {
                                client.send_response(res).await
                            }
                        })
                    }))
                    .await;
                }
            };
            let handle_add_client = {
                let clients = self.clients.clone();
                let client_stream = self.client_stream.clone();
                let next_id = self.next_id.clone();
                async move {
                    let mut client_stream = client_stream.lock().unwrap();
                    if let Some(client) = client_stream.next().await {
                        let mut clients = clients.lock().unwrap();
                        let mut next_id = *next_id.lock().unwrap();
                        if next_id == ClientId::MAX {
                            // TODO notify client sender
                        } else {
                            clients.insert(next_id, client);
                            next_id += 1;
                        }
                    }
                }
            };
            let handle_shutdown = {
                let shutdown_stream = self.shutdown_stream.clone();
                let keep_serving = self.keep_serving.clone();
                async move {
                    let mut shutdown_stream = shutdown_stream.lock().unwrap();
                    let mut keep_serving = keep_serving.lock().unwrap();
                    if let Some(_) = shutdown_stream.next().await {
                        *keep_serving = false
                    }
                }
            };
            futures::future::join3(handle_next_request, handle_add_client, handle_shutdown).await;
        }
    }
}
