use async_std::{
    io::{ReadExt, WriteExt},
    task::{self},
};
use futures::{channel::mpsc, future, stream, AsyncReadExt, FutureExt, SinkExt, StreamExt};
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
            .take(1)
            .for_each_concurrent(None, |stream| async move {
                let stream = ok_or_log_err!(stream);
                let peer_addr = ok_or_log_err!(stream.peer_addr());
                log::debug!("new client: {}", peer_addr);
                let (reader, mut writer) = stream.split();
                let (tx, mut rx) = mpsc::channel(32);
                let tx = Arc::new(Mutex::new(tx));
                let reader_fut = reader
                    .bytes()
                    .inspect(|byte| {
                        let byte = ok_or_log_err!(byte);
                        log::debug!("service got byte: {byte}");
                    })
                    .take_while(|byte| {
                        future::ready(match byte {
                            Ok(3) => false,
                            Ok(_) => true,
                            Err(e) => {
                                log::error!("{e:?}");
                                false
                            }
                        })
                    })
                    .for_each(|byte| {
                        let tx = tx.clone();
                        async move {
                            let mut tx = tx.lock().unwrap();
                            let byte = ok_or_log_err!(byte);
                            ok_or_log_err!(tx.send(byte).await);
                        }
                    })
                    .then(|_| {
                        let tx = tx.clone();
                        async move {
                            let mut tx = tx.lock().unwrap();
                            tx.close_channel();
                        }
                    });
                let writer_fut = async {
                    while let Some(byte) = rx.next().await {
                        log::debug!("echoing byte: {byte}");
                        ok_or_log_err!(writer.write(&[byte]).await);
                    }
                };
                futures::future::join(reader_fut, writer_fut).await;
                log::debug!("done handling client");
            });
        let mut client = async_std::net::TcpStream::connect(listen_addr).await?;
        let client_fut = async move {
            let (tx, mut rx) = mpsc::channel(32);
            let tx = Arc::new(Mutex::new(tx));
            let bytes = [41, 42, 43, 3];
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
                        ok_or_log_err!(client.write(&[byte]).await);
                    }
                }
            };
            futures::future::join(send_fut, recv_fut).await;
            assert_eq!(
                client
                    .bytes()
                    .inspect(|byte| {
                        let byte = ok_or_log_err!(byte);
                        log::debug!("client got byte: {byte}");
                    })
                    .take_while(|byte| {
                        future::ready(match byte {
                            Ok(3) => false,
                            Ok(_) => true,
                            Err(e) => {
                                log::error!("{e:?}");
                                false
                            }
                        })
                    })
                    .map(|b| b.unwrap())
                    .collect::<Vec<u8>>()
                    .await,
                bytes[0..3]
            );
            log::debug!("client done");
        };
        futures::future::join(service_fut, client_fut).await;
        Result::<(), Error>::Ok(())
    })?;
    Ok(())
}
