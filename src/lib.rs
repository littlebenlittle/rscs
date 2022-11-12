use std::sync::mpsc;

trait Client<Req, Res> {
    type Error: std::error::Error;
    fn new(tx: mpsc::Sender<Req>, rx: mpsc::Receiver<Res>) -> Self;
    fn send(&mut self, req: Req) -> Result<(), Self::Error>;
    fn recv(&mut self) -> Result<Res, Self::Error>;
}

trait Service<Req, Res> {
    type Client: Client<Req, Res>;
    type Error: std::error::Error;
    // fn start(&mut self);
    fn client(&mut self) -> Result<Self::Client, Self::Error>;
}

#[cfg(test)]
mod test {

    use std::collections::BTreeMap;

    use super::*;

    enum Request {
        Hello,
    }

    #[derive(Debug, PartialEq)]
    enum Response {
        Hello,
    }

    #[derive(Debug)]
    struct ClientError(String);

    impl std::fmt::Display for ClientError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.0)
        }
    }

    impl std::error::Error for ClientError {}

    impl From<mpsc::SendError<Request>> for ClientError {
        fn from(e: mpsc::SendError<Request>) -> Self {
            Self(format!("{e:?}"))
        }
    }

    impl From<mpsc::RecvError> for ClientError {
        fn from(e: mpsc::RecvError) -> Self {
            Self(format!("{e:?}"))
        }
    }

    struct TestClient {
        tx: mpsc::Sender<Request>,
        rx: mpsc::Receiver<Response>,
    }

    impl Client<Request, Response> for TestClient {
        type Error = ClientError;
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

    #[derive(Debug)]
    struct ServerError(String);

    impl std::fmt::Display for ServerError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.0)
        }
    }

    impl std::error::Error for ServerError {}

    struct TestService {
        next_client_id: u8,
        clients: BTreeMap<u8, (mpsc::Sender<Response>, mpsc::Receiver<Request>)>,
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
        type Client = TestClient;
        type Error = ServerError;
        fn client(&mut self) -> Result<Self::Client, Self::Error> {
            if self.next_client_id == u8::MAX {
                return Err(ServerError("exceeded max clients".to_owned()));
            }
            let client_id = self.next_client_id;
            self.next_client_id += 1;
            let (ctx, srx) = mpsc::channel();
            let (stx, crx) = mpsc::channel();
            self.clients.insert(client_id, (stx, srx));
            Ok(Self::Client::new(ctx, crx))
        }
    }

    #[test]
    fn start_stop_server() -> Result<(), Box<dyn std::error::Error>> {
        let mut svc = TestService::new();
        let mut cli = svc.client()?;
        cli.send(Request::Hello)?;
        assert_eq!(cli.recv()?, Response::Hello);
        Ok(())
    }
}
