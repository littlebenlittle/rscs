use std::{fmt::Debug, io, marker::PhantomData, pin::Pin, sync::Mutex};

use async_std::{
    io::Bytes,
    stream::TakeWhile,
    task::{Context, Poll},
};
use futures::{channel::mpsc, future::Ready, stream, AsyncRead, Future, Stream, StreamExt};

pub trait Parser<I, O, E> {
    fn process_next_item(&mut self, item: I) -> ParseStatus<O, E>;
}

pub enum ParseStatus<T, E> {
    Output(Vec<T>),
    NeedsMore,
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
            items: Mutex::new(Vec::new()),
            _phantom_data: PhantomData,
        }
    }
}

// impl<I, O, E, P> ParseWith<I, O, E, P> for mpsc::Receiver<I> where P: Parser<I, O, E> {}
//
// impl<I, O, E, P, It> ParseWith<I, O, E, P> for stream::Iter<It>
// where
//     P: Parser<I, O, E>,
//     It: Iterator<Item = I>,
// {
// }
//
// impl<O, E, P, R> ParseWith<io::Result<u8>, O, E, P> for Bytes<R>
// where
//     P: Parser<io::Result<u8>, O, E>,
//     R: AsyncRead + Unpin,
// {
// }
//
// impl<I, O, E, P, S, Pred> ParseWith<I, O, E, P> for TakeWhile<S, Pred>
// where
//     S: ParseWith<I, O, E, P>,
//     P: Parser<I, O, E>,
//     Pred: FnMut(&I) -> bool,
// {
// }

impl<I, O, E, P, S> ParseWith<I, O, E, P> for S
where
    S: Stream<Item = I>,
    P: Parser<I, O, E>,
{
}

pub struct ParseWithStream<I, O, E, P, Si>
where
    P: Parser<I, O, E>,
    Si: Stream<Item = I>,
{
    inner: Mutex<Si>,
    parser: Mutex<P>,
    items: Mutex<Vec<O>>,
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
            if let Some(o) = self.items.lock().unwrap().pop() {
                return Poll::Ready(Some(Ok(o)));
            }
            let next_item: I = match self.inner.lock().unwrap().poll_next_unpin(cx) {
                Poll::Ready(Some(t)) => t,
                Poll::Ready(None) => break Poll::Ready(None),
                Poll::Pending => break Poll::Pending,
            };
            use ParseStatus::*;
            match self.parser.lock().unwrap().process_next_item(next_item) {
                Output(mut o) => self.items.lock().unwrap().append(&mut o),
                NeedsMore => {}
                Error(e) => break Poll::Ready(Some(Err(e))),
            }
        }
    }
}
