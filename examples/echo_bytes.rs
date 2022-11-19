use async_std::{
    io::{ReadExt, WriteExt},
    task::{self},
};
use futures::{channel::mpsc, stream, AsyncReadExt, SinkExt, StreamExt};
use std::{
    fmt::Debug,
    io::{Read, Write},
    net::SocketAddr,
    sync::{Arc, Mutex},
};

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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let listen = "127.0.0.1:0".parse::<SocketAddr>()?;
    task::block_on(async {
        let listener = async_std::net::TcpListener::bind(listen).await?;
        let listen_addr = listener.local_addr()?;
        let service_fut = listener
            .incoming()
            .for_each_concurrent(Some(3), |stream| async move {
                let stream = ok_or_log_err!(stream);
                let peer_addr = ok_or_log_err!(stream.peer_addr());
                log::debug!("new client: {}", peer_addr);
                let (reader, mut writer) = stream.split();
                let mut byte_stream = reader.bytes();
                while let Some(byte) = byte_stream.next().await {
                    let byte = ok_or_log_err!(byte);
                    log::debug!("byte: {byte:?}");
                    if byte == 3 {
                        break
                    }
                }
                // let (tx, mut rx) = mpsc::channel(32);
                // let tx = Arc::new(Mutex::new(tx));
                // let reader_fut = reader.bytes().for_each(|byte| {
                //     log::debug!("service got a byte");
                //     let tx = tx.clone();
                //     async move {
                //         let mut tx = tx.lock().unwrap();
                //         let byte = ok_or_log_err!(byte);
                //         log::debug!("service got byte: {byte}");
                //         ok_or_log_err!(tx.send(byte).await);
                //     }
                // });
                // let writer_fut = async {
                //     loop {
                //         if let Some(byte) = rx.next().await {
                //             log::debug!("echoing byte: {byte}");
                //             ok_or_log_err!(writer.write(&[byte]).await);
                //         }
                //     }
                // };
                // log::debug!("awaiting reader/writer futures");
                // futures::future::join(reader_fut, writer_fut).await;
            });
        let mut client = std::net::TcpStream::connect(listen_addr)?;
        let client_fut = async move {
            let (tx, mut rx) = mpsc::channel(32);
            let tx = Arc::new(Mutex::new(tx));
            let bytes = [1, 2, 3];
            let num_bytes = bytes.len();
            let send_fut = stream::iter(bytes).for_each(|byte| {
                let tx = tx.clone();
                async move {
                    let mut tx = tx.lock().unwrap();
                    ok_or_log_err!(tx.send(byte).await);
                }
            });
            let recv_fut = async {
                for _ in 1..=num_bytes {
                    if let Some(byte) = rx.next().await {
                        log::debug!("sending byte: {byte}");
                        ok_or_log_err!( client.write(&[byte]));
                    }
                }
            };
            futures::future::join(send_fut, recv_fut).await;
            assert_eq!(
                client.bytes().map(|b| b.unwrap()).collect::<Vec<u8>>(),
                bytes
            );
        };
        futures::future::join(service_fut, client_fut).await;
        Result::<(), Error>::Ok(())
    })?;
    Ok(())
}
