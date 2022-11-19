use async_std::{
    io::ReadExt,
    task::{self},
};
use futures::StreamExt;
use std::{
    net::SocketAddr,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let listen = "127.0.0.1:0".parse::<SocketAddr>()?;
    task::block_on(async {
        let listener = async_std::net::TcpListener::bind(listen).await?;
        let listen_addr = listener.local_addr()?;
        log::debug!("listening on {listen_addr}");
        listener
            .incoming()
            .for_each_concurrent(Some(3), |stream| async move {
                let mut stream = stream.unwrap();
                log::debug!("new client: {}", stream.peer_addr().unwrap());
                let mut buf = [0u8; 1];
                while stream.read(&mut buf).await.is_ok() {
                    log::debug!("buf: {buf:?}");
                }
                log::debug!("done processing client");
            })
            .await;
        Result::<(), std::io::Error>::Ok(())
    })?;
    Ok(())
}
