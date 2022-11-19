use async_std::{
    io::{ReadExt, WriteExt},
    task,
};
use futures::{channel::mpsc, future, stream, AsyncReadExt, FutureExt, SinkExt, StreamExt};
use std::{
    fmt::Debug,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use rscs::{ParseStatus, ParseWith, Parser};

#[derive(Debug, PartialEq)]
enum Message {
    Noop,
    Echo(String),
}

struct MessageEncoder {
    more: Option<Vec<u8>>,
    more_index: usize,
}

impl MessageEncoder {
    fn new() -> Self {
        Self {
            more: Some(Vec::new()),
            more_index: 0,
        }
    }
    fn encode(msg: Message) -> Vec<u8> {
        match msg {
            Message::Noop => vec![0, 0],
            Message::Echo(s) => {
                let mut bytes = Vec::new();
                bytes.push(1u8);
                bytes.extend(s.into_bytes());
                bytes.push(0);
                bytes
            }
        }
    }
}

impl Parser<Message, u8, Error> for MessageEncoder {
    fn process_next_item(&mut self, item: Message) -> ParseStatus<u8, Error> {
        let bytes = Self::encode(item);
        if bytes.len() > 1 {
            self.more = Some(bytes[1..].to_vec());
            ParseStatus::Output(bytes[0])
        } else {
            ParseStatus::Output(bytes[0])
        }
    }
    fn get_more(&mut self) -> Option<u8> {
        let more = self.more.take();
        if let Some(more) = more {
            self.more_index += 1;
            if self.more_index >= more.len() {
                self.more_index = 0;
                return None;
            }
            let byte = more[self.more_index];
            self.more = Some(more);
            return Some(byte);
        }
        return None;
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
                0 => ParseStatus::Output(Message::Noop),
                _ => {
                    self.context.push(item);
                    ParseStatus::NeedsMore
                }
            },
            Some(_) => match item {
                0 => match String::from_utf8(self.context.clone()) {
                    Ok(s) => ParseStatus::Output(Message::Echo(s)),
                    Err(e) => ParseStatus::Error(Error(e.to_string())),
                },
                _ => {
                    self.context.push(item);
                    ParseStatus::NeedsMore
                }
            },
        }
    }

    fn get_more(&mut self) -> Option<Message> {
        None
    }
}

impl Parser<std::io::Result<u8>, Message, Error> for MessageDecoder {
    fn process_next_item(&mut self, item: std::io::Result<u8>) -> ParseStatus<Message, Error> {
        match item {
            Ok(item) => Parser::<u8, Message, Error>::process_next_item(self, item),
            Err(e) => ParseStatus::Error(e.into()),
        }
    }
    fn get_more(&mut self) -> Option<Message> {
        None
    }
}

#[derive(Debug, PartialEq)]
struct Error(String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self(e.to_string())
    }
}

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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let listen = "127.0.0.1:0".parse::<SocketAddr>()?;
    task::block_on(async {
        let listener = async_std::net::TcpListener::bind(listen).await?;
        let listen_addr = listener.local_addr()?;
        let service_fut = listener
            .incoming()
            .take(1) // otherwise service keeps running
            .for_each_concurrent(None, |stream| async move {
                let stream = ok_or_log_err!(stream);
                let peer_addr = ok_or_log_err!(stream.peer_addr());
                log::debug!("new client: {}", peer_addr);
                let (reader, mut writer) = stream.split();
                let (tx, rx) = mpsc::channel(32);
                let tx = Arc::new(Mutex::new(tx));
                let reader_fut = reader
                    .bytes()
                    .inspect(|byte| {
                        let byte = ok_or_log_err!(byte);
                        log::debug!("service got byte: {byte}");
                    })
                    .take_while(|byte| take_unless!(byte, 3))
                    .parse_with(MessageDecoder::new())
                    .inspect(|req| {
                        let req = ok_or_log_err!(req);
                        log::debug!("service got req: {req:?}");
                    })
                    .for_each(|req| {
                        async_lock!(tx, {
                            let req = ok_or_log_err!(req);
                            ok_or_log_err!(tx.send(req).await);
                        })
                    });
                let writer_fut = async {
                    let mut chunk_stream = rx
                        .inspect(|req| {
                            log::debug!("service sending req: {req:?}");
                        })
                        .parse_with(MessageEncoder::new())
                        .take_while(|byte| future::ready(byte.is_ok()))
                        .map(|byte| byte.unwrap())
                        .ready_chunks(16);
                    while let Some(bytes) = chunk_stream.next().await {
                        log::debug!("service sending bytes: {bytes:?}");
                        ok_or_log_err!(writer.write_all(&bytes).await);
                    }
                };
                futures::future::join(reader_fut, writer_fut).await;
                log::debug!("done handling client");
            });
        let mut client = async_std::net::TcpStream::connect(listen_addr).await?;
        let client_fut = async move {
            let (tx, mut rx) = mpsc::channel(32);
            let tx = Arc::new(Mutex::new(tx));
            let msgs = [Message::Noop, Message::Echo("test".into())];
            let encode_fut = stream::iter(msgs)
                .parse_with(MessageEncoder::new())
                .take_while(|byte| future::ready(byte.is_ok()))
                .map(|byte| byte.unwrap())
                .ready_chunks(16)
                .for_each(|bytes| async_lock!(tx, ok_or_log_err!(tx.send(bytes).await)))
                .then(|_| async { log::debug!("done encoding messages") });
            let send_fut = async {
                if let Some(bytes) = rx.next().await {
                    log::debug!("sending bytes: {bytes:?}");
                    ok_or_log_err!(client.write_all(&bytes).await);
                }
                log::debug!("done sending bytes");
            };
            futures::future::join(encode_fut, send_fut).await;
            log::debug!("receiving bytes on client");
            client
                .bytes()
                .for_each(|byte| async move {
                    let byte = ok_or_log_err!(byte);
                    log::debug!("client got byte: {byte}");
                })
                .await;
        };
        futures::future::join(service_fut, client_fut).await;
        Result::<(), Error>::Ok(())
    })?;
    Ok(())
}
