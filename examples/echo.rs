use async_std::{
    io::{ReadExt, WriteExt},
    task::{self},
};
use futures::{channel::mpsc, stream, AsyncReadExt, SinkExt, StreamExt};
use std::{
    fmt::Debug,
    io::Write,
    net::{SocketAddr, TcpStream},
    sync::{Arc, Mutex},
};

use rscs::{ParseStatus, ParseWith, Parser};

fn take_ok_or_log_err<T, E: Debug>(r: Result<T, E>) -> Option<T> {
    match r {
        Ok(b) => Some(b),
        Err(e) => {
            log::error!("{e:?}");
            None
        }
    }
}

#[derive(Debug, PartialEq)]
enum Message {
    Noop,
    Echo(String),
}

#[derive(Debug, PartialEq)]
struct Error(String);

struct MessageEncoder {}

impl MessageEncoder {
    fn new() -> Self {
        Self {}
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

impl Parser<Message, Vec<u8>, Error> for MessageEncoder {
    fn process_next_item(&mut self, item: Message) -> ParseStatus<Vec<u8>, Error> {
        ParseStatus::Finished(Self::encode(item))
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
                ParseStatus::Incomplete
            }
            Some(0) => match item {
                0 => ParseStatus::Finished(Message::Noop),
                _ => {
                    self.context.push(item);
                    ParseStatus::Incomplete
                }
            },
            Some(_) => match item {
                0 => match String::from_utf8(self.context.clone()) {
                    Ok(s) => ParseStatus::Finished(Message::Echo(s)),
                    Err(e) => ParseStatus::Error(Error(e.to_string())),
                },
                _ => {
                    self.context.push(item);
                    ParseStatus::Incomplete
                }
            },
        }
    }
}

impl Parser<std::io::Result<u8>, Message, Error> for MessageDecoder {
    fn process_next_item(
        &mut self,
        item: std::io::Result<u8>,
    ) -> ParseStatus<Message, Error> {
        match item {
            Ok(item) => Parser::<u8, Message, Error>::process_next_item(self, item),
            Err(e) => ParseStatus::Error(e.into()),
        }
    }
}

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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let listen = "127.0.0.1:0".parse::<SocketAddr>()?;
    task::block_on(async {
        let listener = async_std::net::TcpListener::bind(listen).await?;
        let listen_addr = listener.local_addr()?;
        let jh = task::spawn(async move {
            listener
                .incoming()
                .for_each_concurrent(Some(3), |stream| async move {
                    let stream = match stream {
                        Ok(s) => s,
                        Err(e) => {
                            log::error!("{e:?}");
                            return;
                        }
                    };
                    let (reader, writer) = stream.split();
                    reader
                        .bytes()
                        .parse_with(MessageDecoder::new()) // magic ✨
                        .for_each(|req| async move {
                            match req {
                                Ok(req) => match writer.write(&MessageEncoder::encode(req)).await {
                                    Ok(_) => {}
                                    Err(e) => log::error!("{e:?}"),
                                },
                                Err(e) => log::error!("{e:?}"),
                            }
                        })
                        .await
                });
            Result::<(), Error>::Ok(())
        });
        let client = std::net::TcpStream::connect(listen_addr)?;
        stream::iter([Message::Noop, Message::Echo("test".into())])
            .parse_with(MessageEncoder::new()) // magic ✨
            .for_each(|bytes| async move {
                if let Some(bytes) = take_ok_or_log_err(bytes) {
                    client.write_all(&bytes);
                }
            })
            .await;
        jh.cancel().await;
        Result::<(), Error>::Ok(())
    })?;
    Ok(())
}
