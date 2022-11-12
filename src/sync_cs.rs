use std::sync::mpsc;

pub trait Client<Req, Res> {
    type Error;
    fn new(tx: mpsc::Sender<Req>, rx: mpsc::Receiver<Res>) -> Self;
    fn send(&mut self, req: Req) -> Result<(), Self::Error>;
    fn recv(&mut self) -> Result<Res, Self::Error>;
}

pub trait ClientHandler<Req, Res, Svc: Service<Req, Res>> {
    type Error;
    fn send(&mut self, res: Res) -> Result<(), Self::Error>;
    fn recv(&mut self) -> Result<Option<Req>, Self::Error>;
}

pub trait Service<Req, Res>: Sized {
    type Client: Client<Req, Res>;
    type Error: From<<Self::ClientHandler as ClientHandler<Req, Res, Self>>::Error>;
    type ClientHandler: ClientHandler<Req, Res, Self>;

    fn client(&mut self) -> Result<Self::Client, Self::Error>;
    fn process_next_request(&mut self) -> Result<(), Self::Error>;
}

#[cfg(test)]
mod test {

    use std::collections::BTreeMap;

    use super::*;
    use crate::test_fixtures::{Request, Response, Error};

    impl From<mpsc::SendError<Request>> for Error {
        fn from(e: mpsc::SendError<Request>) -> Self {
            Self(format!("{e:?}"))
        }
    }

    impl From<mpsc::SendError<Response>> for Error {
        fn from(e: mpsc::SendError<Response>) -> Self {
            Self(format!("{e:?}"))
        }
    }

    impl From<mpsc::RecvError> for Error {
        fn from(e: mpsc::RecvError) -> Self {
            Self(format!("{e:?}"))
        }
    }

    struct TestClient {
        tx: mpsc::Sender<Request>,
        rx: mpsc::Receiver<Response>,
    }

    impl Client<Request, Response> for TestClient {
        type Error = Error;
        fn new(tx: mpsc::Sender<Request>, rx: mpsc::Receiver<Response>) -> Self {
            Self { tx, rx }
        }
        fn send(&mut self, req: Request) -> Result<(), Self::Error> {
            Ok(self.tx.send(req)?)
        }
        fn recv(&mut self) -> Result<Response, Self::Error> {
            Ok(self.rx.recv()?)
        }
    }

    struct TestClientHandler {
        tx: mpsc::Sender<Response>,
        rx: mpsc::Receiver<Request>,
    }

    impl ClientHandler<Request, Response, TestService> for TestClientHandler {
        type Error = Error;
        fn send(&mut self, res: Response) -> Result<(), Self::Error> {
            Ok(self.tx.send(res)?)
        }
        fn recv(&mut self) -> Result<Option<Request>, Self::Error> {
            Ok(self.rx.iter().next())
        }
    }

    struct TestService {
        next_client_id: u8,
        clients: BTreeMap<u8, TestClientHandler>,
    }

    impl TestService {
        fn new() -> Self {
            Self {
                next_client_id: 0,
                clients: BTreeMap::new(),
            }
        }
    }

    impl Service<Request, Response> for TestService {
        type Error = Error;
        type Client = TestClient;
        type ClientHandler = TestClientHandler;

        fn client(&mut self) -> Result<Self::Client, Self::Error> {
            if self.next_client_id == u8::MAX {
                return Err(Error("exceeded max clients".to_owned()));
            }
            let client_id = self.next_client_id;
            self.next_client_id += 1;
            let (ctx, srx) = mpsc::channel();
            let (stx, crx) = mpsc::channel();
            self.clients
                .insert(client_id, TestClientHandler { tx: stx, rx: srx });
            Ok(Self::Client::new(ctx, crx))
        }

        fn process_next_request(&mut self) -> Result<(), Self::Error> {
            for (_id, client) in &mut self.clients {
                match client.recv()? {
                    Some(Request::A) => {
                        client.send(Response::A)?;
                        break;
                    }
                    Some(Request::B) => {
                        client.send(Response::B)?;
                        break;
                    }
                    None => {}
                }
            }
            Ok(())
        }
    }

    #[test]
    fn process_one_request() -> Result<(), Box<dyn std::error::Error>> {
        let mut svc = TestService::new();
        let mut cli = svc.client()?;
        cli.send(Request::A)?;
        svc.process_next_request()?;
        assert_eq!(cli.recv()?, Response::A);
        Ok(())
    }

    #[test]
    fn process_two_requests() -> Result<(), Box<dyn std::error::Error>> {
        let mut svc = TestService::new();
        let mut cli = svc.client()?;
        cli.send(Request::A)?;
        cli.send(Request::B)?;
        svc.process_next_request()?;
        svc.process_next_request()?;
        assert_eq!(cli.recv()?, Response::A);
        assert_eq!(cli.recv()?, Response::B);
        Ok(())
    }

    #[test]
    fn process_two_requests_sequentially() -> Result<(), Box<dyn std::error::Error>> {
        let mut svc = TestService::new();
        let mut cli = svc.client()?;
        cli.send(Request::A)?;
        cli.send(Request::B)?;
        svc.process_next_request()?;
        assert_eq!(cli.recv()?, Response::A);
        svc.process_next_request()?;
        assert_eq!(cli.recv()?, Response::B);
        Ok(())
    }
}
