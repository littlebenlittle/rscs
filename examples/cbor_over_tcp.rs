use std::collections::BTreeMap;
use std::net::IpAddr;

use std::net::TcpStream;
use async_std::net::TcpListener;
use futures::StreamExt;
use rscs::{Service, ServiceData};
use serde::Deserialize;
use serde::Serialize;

type ClientId = u8;
type Value = String;
type ErrMsg = String;

#[derive(Serialize, Deserialize)]
enum Request {
    SetValue(Value),
    GetValue,
}

#[derive(Serialize, Deserialize)]
enum Response {
    SetValueOk,
    SetValueErr(ErrMsg),
    GetValueOk(Value),
    GetValueErr(ErrMsg),
}

struct Data {
    listen: IpAddr,
    client_data: BTreeMap<ClientId, String>,
}

impl ServiceData<Request, Response> for Data {
    fn process_request(&mut self, req: Request) -> Option<Response> {
        match req {
            Request::SetValue(val) => {
                self.client_data.insert(client_id, val);
                Some(Response::SetValueOk)
            }
            Request::GetValue => Some(Response::GetValueOk(
                self.client_data.get(client_id).unwrap(),
            )),
        }
    }
}

struct TcpClient {
    connection: TcpStream,
}

impl TcpClient {
    fn dial(addr: IpAddr) -> Result<Self, String> {
        Ok(Self {
            connection: TcpStream::connect(addrs)?
        })
    }
    fn send(&mut self, req: Request) -> Result<(), Error> {
        serde_cbor::to_writer(self.connection, &req).into()
    }
    fn recv(&mut self) -> Result<Response, Error> {
        serde_cbor::from_reader(self.connection).into()
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let svc = Service<TcpClient>::new(Data {
        listen: "127.0.0.1:0".parse(),
        client_data: BTreeMap::new(),
    });
    let shutdown = svc.shutdown();
    let add_client = svc.add_client();
    let jh = task::spawn(async move { 
        let listener = TcpListener::bind(self.listen).await?;
        add_client(
            listener.incoming().map(|stream| {
                Client{ connection: stream.unwrap()};
            })
        ).await;
    });
    task::block_on(async move {
        let cli_1 = TcpClient::dial(svc.listen_addr());
        let cli_2 = TcpClient::dial(svc.listen_addr());
        cli_1.send(Request::SetValue("A!".into())).await?;
        cli_2.send(Request::SetValue("B!".into())).await?;
        assert_eq!(cli_1.recv().await, Some(Response::SetValueOk));
        assert_eq!(cli_2.recv().await, Some(Response::SetValueOk));
        cli_1.send(Request::GetValue("A!")).await?;
        cli_2.send(Request::GetValue("B!")).await?;
        assert_eq!(cli_1.recv().await, Some(Response::GetValue("A!")));
        assert_eq!(cli_2.recv().await, Some(Response::GetValue("B!")));
        shutdown.await?;
        jh.await?;
        Result::<(), Error>::Ok(())
    })?;
    Ok(())
}
