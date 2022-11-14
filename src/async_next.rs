use std::{
    marker::PhantomData,
    sync::{Arc, Mutex}, collections::BTreeMap,
};

pub struct Error(String);

impl Error {
    fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
}

type ClientId = u32;

pub trait Client {}

pub trait ServiceData {}

pub struct Service<C: Client> {
    _phantom_data: PhantomData<C>,
}

impl<C: Client> Service<C> {
    fn new<D: ServiceData>(data: D) -> ServiceMgr<C, D> {
        ServiceMgr {
            data: Arc::new(Mutex::new(data)),
            clients: Arc::new(Mutex::new(BTreeMap::new())),
            next_id: Arc::new(Mutex::new(0))
        }
    }
}

pub struct ServiceMgr<C: Client, D: ServiceData> {
    data: Arc<Mutex<D>>,
    clients: Arc<Mutex<BTreeMap<ClientId, C>>>,
    next_id: Arc<Mutex<ClientId>>,
}

impl<C: Client, D: ServiceData> ServiceMgr<C, D> {
    pub async fn add_client(&mut self, client: C) -> Result<(), Error> {
        let mut clients = self.clients.lock().unwrap();
        let mut next_id = *self.next_id.lock().unwrap();
        if next_id == ClientId::MAX {
            return Err(Error::new("num clients exceeds u32::MAX"))
        }
        clients.insert(next_id, client);
        next_id += 1;
        Ok(())
    }
    
    pub async fn serve(&mut self) -> Result<
}
