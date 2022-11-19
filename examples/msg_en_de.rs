use async_std::task;
use futures::{stream, StreamExt};
use std::fmt::Debug;

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
    fn encode(msg: Message) -> Vec<u8> {
        match msg {
            Message::Noop => vec![0, 0],
            Message::Echo(s) => {
                let mut bytes = Vec::new();
                bytes.push(1u8);
                bytes.extend(s.into_bytes());
                bytes.push(0);
                bytes.reverse();
                bytes
            }
        }
    }
}

impl Parser<Message, u8, Error> for MessageEncoder {
    fn process_next_item(&mut self, item: Message) -> ParseStatus<u8, Error> {
        ParseStatus::Output(Self::encode(item))
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
                log::debug!("byte without context: {item}");
                self.context.push(item);
                ParseStatus::NeedsMore
            }
            Some(0) => {
                log::debug!("byte with context: {item}, {:?}", self.context);
                match item {
                    0 => {
                        self.context.drain(..);
                        ParseStatus::Output(vec![Message::Noop])
                    }
                    _ => {
                        self.context.push(item);
                        ParseStatus::NeedsMore
                    }
                }
            }
            Some(_) => {
                log::debug!("byte with context: {item}, {:?}", self.context);
                match item {
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
                }
            }
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

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self(e.to_string())
    }
}

impl std::error::Error for Error {}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    task::block_on(async {
        let msgs = [
            Message::Noop,
            Message::Echo("test".into()),
            Message::Echo("other".into()),
            Message::Noop,
            Message::Noop,
            Message::Echo("blah blah blah".into()),
        ];
        stream::iter(msgs.clone())
            .inspect(|msg| log::debug!("encoding msg: {msg:?}"))
            .parse_with(MessageEncoder::new())
            .map(|byte| byte.unwrap())
            .inspect(|byte| log::debug!("encoded byte: {byte}"))
            .parse_with(MessageDecoder::new())
            .inspect(|msg| log::debug!("decoded msg: {msg:?}"))
            .map(|msg| msg.unwrap())
            .zip(stream::iter(msgs))
            .for_each(|(got, exp)| async move { assert_eq!(got, exp) })
            .await;
    });
    Ok(())
}
