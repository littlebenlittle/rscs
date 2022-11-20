use async_std::{
    io::{ReadExt, WriteExt},
    net::{TcpListener, TcpStream},
    task,
};
use futures::{
    channel::mpsc,
    future,
    io::{ReadHalf, WriteHalf},
    AsyncReadExt, Future, FutureExt, SinkExt, StreamExt,
};
use std::{
    fmt::Debug,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use rscs::{ParseStatus, ParseWith, Parser};

#[derive(Debug, PartialEq, Clone)]
enum Message {
    Noop,
    Echo(String),
}

struct MessageEncoder {}

impl MessageEncoder {
    fn new() -> Self {
        Self {}
    }
    fn encode(msg: &Message) -> Vec<u8> {
        match msg {
            Message::Noop => vec![0, 0],
            Message::Echo(s) => {
                let mut bytes = Vec::new();
                bytes.push(1u8);
                bytes.extend(s.as_bytes());
                bytes.push(0);
                bytes.reverse();
                bytes
            }
        }
    }
}

impl Parser<Message, u8, Error> for MessageEncoder {
    fn process_next_item(&mut self, item: Message) -> ParseStatus<u8, Error> {
        ParseStatus::Output(Self::encode(&item))
    }
}

struct MessageDecoder {
    context: Vec<u8>,
}

impl MessageDecoder {
    fn new() -> Self {
        Self {
            context: Default::default(),
        }
    }
}

impl Parser<u8, Message, Error> for MessageDecoder {
    fn process_next_item(&mut self, item: u8) -> ParseStatus<Message, Error> {
        match self.context.last() {
            None => {
                self.context.push(item);
                ParseStatus::NeedsMore
            }
            Some(0) => match item {
                0 => {
                    self.context.drain(..);
                    ParseStatus::Output(vec![Message::Noop])
                }
                _ => {
                    self.context.push(item);
                    ParseStatus::NeedsMore
                }
            },
            Some(_) => match item {
                0 => {
                    let s = self.context.drain(..).collect::<Vec<u8>>();
                    match String::from_utf8(s[1..].to_vec()) {
                        Ok(s) => ParseStatus::Output(vec![Message::Echo(s)]),
                        Err(e) => ParseStatus::Error(Error(e.to_string())),
                    }
                }
                _ => {
                    self.context.push(item);
                    ParseStatus::NeedsMore
                }
            },
        }
    }
}

impl Parser<std::io::Result<u8>, Message, Error> for MessageDecoder {
    fn process_next_item(&mut self, item: std::io::Result<u8>) -> ParseStatus<Message, Error> {
        match item {
            Ok(item) => Parser::<u8, Message, Error>::process_next_item(self, item),
            Err(e) => ParseStatus::Error(e.into()),
        }
    }
}

#[derive(Debug, PartialEq)]
struct Error(String);

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

impl std::error::Error for Error {}

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

macro_rules! async_lock {
    ($x: ident, $y: block) => {{
        let $x = $x.clone();
        async move {
            let mut $x = $x.lock().unwrap();
            $y
        }
    }};

    ($x: ident, $y: expr) => {{
        let $x = $x.clone();
        async move {
            let mut $x = $x.lock().unwrap();
            $y
        }
    }};
}

macro_rules! take_unless {
    ($byte: ident, $($ns:literal),+) => {
        future::ready(match $byte {
            $(
            Ok($ns) => false,
            ),+
            Ok(_) => true,
            Err(e) => {
                log::error!("{e:?}");
                false
            }
        })
    };
}

type Result<T> = std::result::Result<T, Error>;
struct Client {
    reader: ReadHalf<TcpStream>,
    writer: WriteHalf<TcpStream>,
    req_tx: Arc<Mutex<mpsc::Sender<Message>>>,
    req_rx: mpsc::Receiver<Message>,
    res_tx: Arc<Mutex<mpsc::Sender<Result<Message>>>>,
    res_rx: Option<mpsc::Receiver<Result<Message>>>,
}

impl Client {
    async fn new(listen: SocketAddr) -> Result<Self> {
        let (res_tx, res_rx) = mpsc::channel(0);
        let (req_tx, req_rx) = mpsc::channel(0);
        let (reader, writer) = TcpStream::connect(listen).await?.split();
        Ok(Self {
            reader,
            writer,
            req_tx: Arc::new(Mutex::new(req_tx)),
            req_rx,
            res_tx: Arc::new(Mutex::new(res_tx)),
            res_rx: Some(res_rx),
        })
    }

    fn sender(&mut self) -> ClientSender {
        ClientSender {
            tx: self.req_tx.clone(),
        }
    }

    fn receiver(&mut self) -> ClientReceiver {
        ClientReceiver {
            rx: self.res_rx.take().expect("can only take receiver once"),
        }
    }

    async fn process(mut self) -> Result<()> {
        let read_fut = async {
            let mut res_stream = self
                .reader
                .bytes()
                .take_while(|byte| take_unless!(byte, 3))
                .parse_with(MessageDecoder::new());
            while let Some(msg) = res_stream.next().await {
                self.res_tx.lock().unwrap().send(msg).await?;
            }
            Result::<()>::Ok(())
        };
        let write_fut = async {
            while let Some(msg) = self.req_rx.next().await {
                self.writer.write_all(&MessageEncoder::encode(&msg)).await?;
            }
            Result::<()>::Ok(())
        };
        let (r1, r2) = future::join(read_fut, write_fut).await;
        r1?;
        r2?;
        Ok(())
    }
}

struct ClientSender {
    tx: Arc<Mutex<mpsc::Sender<Message>>>,
}

impl ClientSender {
    async fn send(&mut self, msg: Message) -> Result<()> {
        Ok(self.tx.lock().unwrap().send(msg).await?)
    }
}

struct ClientReceiver {
    rx: mpsc::Receiver<Result<Message>>,
}

impl ClientReceiver {
    async fn recv(&mut self) -> Option<Result<Message>> {
        self.rx.next().await
    }
}

struct Service {
    listener: TcpListener,
    shutdown_tx: Arc<Mutex<mpsc::Sender<()>>>,
    shutdown_rx: mpsc::Receiver<()>,
}

impl Service {
    async fn new(listen: SocketAddr) -> Result<Self> {
        let (shutdown_tx, shutdown_rx) = mpsc::channel(0);
        Ok(Self {
            listener: TcpListener::bind(listen).await?,
            shutdown_tx: Arc::new(Mutex::new(shutdown_tx)),
            shutdown_rx,
        })
    }

    async fn client(&mut self) -> Result<Client> {
        Ok(Client::new(self.listener.local_addr()?).await?)
    }

    fn shutdown(&mut self) -> impl Future<Output = Result<()>> {
        let tx = self.shutdown_tx.clone();
        async move { Ok(tx.lock().unwrap().send(()).await?) }
    }

    async fn serve(mut self) {
        self.listener
            .incoming()
            .take_until(self.shutdown_rx.next())
            .for_each_concurrent(None, |stream| async move {
                let stream = ok_or_log_err!(stream);
                let peer_addr = ok_or_log_err!(stream.peer_addr());
                log::debug!("new client: {}", peer_addr);
                let (reader, mut writer) = stream.split();
                let (tx, rx) = mpsc::channel(32);
                let tx = Arc::new(Mutex::new(tx));
                let reader_fut = reader
                    .bytes()
                    .take_while(|byte| take_unless!(byte, 3))
                    .parse_with(MessageDecoder::new())
                    .inspect(|msg| {
                        let msg = ok_or_log_err!(msg);
                        log::debug!("service got msg: {msg:?}");
                    })
                    .for_each(|msg| {
                        async_lock!(tx, {
                            let msg = ok_or_log_err!(msg);
                            ok_or_log_err!(tx.send(msg).await);
                        })
                    })
                    .then(|_| async_lock!(tx, tx.close_channel()));
                let writer_fut = async {
                    let mut chunk_stream = rx
                        .inspect(|msg| log::debug!("service sending msg: {msg:?}"))
                        .parse_with(MessageEncoder::new())
                        .take_while(|byte| future::ready(byte.is_ok()))
                        .map(|byte| byte.unwrap())
                        .ready_chunks(16);
                    while let Some(bytes) = chunk_stream.next().await {
                        log::debug!("service sending bytes: {bytes:?}");
                        ok_or_log_err!(writer.write_all(&bytes).await);
                    }
                    log::debug!("service sending EOT");
                    ok_or_log_err!(writer.write_all(&[3]).await);
                };
                futures::future::join(reader_fut, writer_fut).await;
                log::debug!("done handling client");
            })
            .await
    }
}

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let listen = "127.0.0.1:0".parse::<SocketAddr>()?;
    task::block_on(async {
        let mut service = Service::new(listen).await?;
        let mut client = service.client().await?;
        let shutdown = service.shutdown();
        let client_fut = async move {
            let mut sender = client.sender();
            let mut receiver = client.receiver();
            let send_fut = async move {
                let msgs = [
                    Message::Noop,
                    Message::Echo("test".into()),
                    Message::Noop,
                    Message::Echo("somthing".into()),
                    Message::Echo("else".into()),
                    Message::Noop,
                ];
                for msg in msgs {
                    match sender.send(msg.clone()).await {
                        Ok(()) => {}
                        Err(e) => log::error!("{e:?}"),
                    }
                    assert_eq!(receiver.recv().await, Some(Ok(msg)));
                }
            };
            let (r1, _r2) = futures::future::join(client.process(), send_fut).await;
            r1.unwrap();
            shutdown.await.unwrap();
        };
        futures::future::join(client_fut, service.serve()).await;
        Result::<()>::Ok(())
    })?;
    Ok(())
}
