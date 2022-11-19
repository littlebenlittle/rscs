use async_std::{
    io::ReadExt,
    task::{self},
};
use futures::{future, StreamExt};
use std::{io::Write, net::SocketAddr};

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
    let listen = "127.0.0.1:41715".parse::<SocketAddr>()?;
    task::block_on(async {
        let listener = async_std::net::TcpListener::bind(listen).await?;
        let listen_addr = listener.local_addr()?;
        log::debug!("listening on {listen_addr}");
        let service_fut = listener
            .incoming()
            .for_each_concurrent(None, |stream| async move {
                let stream = stream.unwrap();
                log::debug!("new client: {}", stream.peer_addr().unwrap());
                stream
                    .bytes()
                    .take_while(|byte| match byte {
                        Ok(3) => future::ready(false),
                        Ok(_) => future::ready(true),
                        Err(e) => {
                            log::error!("{e:?}");
                            future::ready(false)
                        }
                    })
                    .for_each(|byte| async move {
                        let byte = ok_or_log_err!(byte);
                        log::debug!("byte: {byte}");
                    })
                    .await;
                log::debug!("done handling client");
            });
        let mut client = std::net::TcpStream::connect(listen_addr)?;
        let client_fut = async move {
            let bytes = [1, 2, 3];
            for byte in bytes {
                log::debug!("sending byte: {byte}");
                client.write(&[byte]).unwrap();
            }
            log::debug!("done running client");
        };
        futures::future::join(service_fut, client_fut).await;
        Result::<(), std::io::Error>::Ok(())
    })?;
    Ok(())
}
