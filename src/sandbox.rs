use async_std::task;
use async_std::{io::ReadExt, net::TcpStream};
use async_trait::async_trait;
use futures::{
    channel::mpsc::{self, SendError},
    AsyncReadExt, AsyncWriteExt, Future, SinkExt, Stream, StreamExt,
};
use std::{
    collections::BTreeMap,
    marker::PhantomData,
    pin::Pin,
    sync::{Arc, Mutex},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let jh = task::spawn(async {
        let listener = async_std::net::TcpListener::bind("127.0.0.1:0").await?;
        listener
            .incoming()
            .for_each_concurrent(None, |stream| async move {
                let stream = match stream {
                    Ok(s) => s,
                    Err(e) => {
                        log::error!("{e:?}");
                        return;
                    }
                };
                let (reader, writer) = stream.split();
                let writer = Arc::new(Mutex::new(writer));
                reader
                    .bytes()
                    .filter_map(|b| async move {
                        match b {
                            Ok(b) => Some(b),
                            Err(e) => {
                                log::error!("e:?");
                                None
                            }
                        }
                    })
                    .ready_chunks(8)
                    .for_each(|chunk| {
                        let writer = writer.clone();
                        async move {
                            let mut writer = writer.lock().unwrap();
                            match writer.write_all(&chunk).await {
                                Err(e) => log::error!("failed to write to stream: {e:?}"),
                                _ => {}
                            }
                        }
                    })
                    .await;
            })
            .await;
        Result::<(), std::io::Error>::Ok(())
    });
    task::block_on(async move { jh.await })?;
    Ok(())
}
