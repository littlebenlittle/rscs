use std::{io, marker::PhantomData, pin::Pin, sync::Mutex};

use async_std::{
    io::Bytes,
    task::{Context, Poll},
};
use futures::{channel::mpsc, stream, AsyncRead, Stream, StreamExt};

pub trait Parser<I, O, E> {
    fn process_next_item(&mut self, item: I) -> ParseStatus<O, E>;
}

pub enum ParseStatus<T, E> {
    Finished(T),
    Incomplete,
    Error(E),
}

pub trait ParseWith<I, O, E, P>: Stream<Item = I> + Sized
where
    P: Parser<I, O, E>,
{
    fn parse_with(self, parser: P) -> ParseWithStream<I, O, E, P, Self> {
        ParseWithStream {
            inner: Mutex::new(self),
            parser: Mutex::new(parser),
            _phantom_data: PhantomData,
        }
    }
}

impl<I, O, E, P> ParseWith<I, O, E, P> for mpsc::Receiver<I> where P: Parser<I, O, E> {}

impl<I, O, E, P, It> ParseWith<I, O, E, P> for stream::Iter<It>
where
    P: Parser<I, O, E>,
    It: Iterator<Item = I>,
{
}

impl<O, E, P, S> ParseWith<io::Result<u8>, O, E, P> for Bytes<S>
where
    P: Parser<io::Result<u8>, O, E>,
    S: AsyncRead + Unpin,
{
}

pub struct ParseWithStream<I, O, E, P, Si>
where
    P: Parser<I, O, E>,
    Si: Stream<Item = I>,
{
    inner: Mutex<Si>,
    parser: Mutex<P>,
    _phantom_data: PhantomData<(O, E)>,
}

impl<I, O, E, P, Si> Stream for ParseWithStream<I, O, E, P, Si>
where
    P: Parser<I, O, E>,
    Si: Stream<Item = I> + Unpin,
{
    type Item = Result<O, E>;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            let next_item: I = match self.inner.lock().unwrap().poll_next_unpin(cx) {
                Poll::Ready(Some(t)) => t,
                Poll::Ready(None) => break Poll::Ready(None),
                Poll::Pending => break Poll::Pending,
            };
            match self.parser.lock().unwrap().process_next_item(next_item) {
                ParseStatus::Finished(o) => {
                    break Poll::Ready(Some(Ok(o)));
                }
                ParseStatus::Incomplete => {}
                ParseStatus::Error(e) => break Poll::Ready(Some(Err(e))),
            }
        }
    }
}
