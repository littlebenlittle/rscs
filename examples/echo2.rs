use std::{marker::PhantomData, pin::Pin, sync::Mutex};

use async_std::task::{self, Context, Poll};
use futures::{channel::mpsc, SinkExt, Stream, StreamExt};

// I am trying to learn more about async rust. My current project is building a
// stream combinator that converts a stream of one type into a stream that reads
// one or more items from the underlying stream and produces items of a different
// type built from those items.
//
// My working case study is a parser that reads from a byte stream and produces
// a stream of CStrings, or null-terminated sequences of bytes. I would like the
// API to look something like this:

/// A null-terminated sequence of bytes
#[derive(Debug, PartialEq)]
struct CString(Vec<u8>);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    // spawn is a more general setting than block_on
    let jh = task::spawn(async {
        log::debug!("creating channels");
        let (mut tx, rx) = mpsc::channel(16);
        log::debug!("converting stream");
        let mut stream = rx.parse_with(CStringParser::new());
        for c in [1, 2, 3, 4, 0, 4, 3, 2, 1, 0] {
            log::debug!("sending byte: {c}");
            tx.send(c).await?;
        }
        assert_eq!(stream.next().await, Some(CString(vec!(1, 2, 3, 4, 0))));
        assert_eq!(stream.next().await, Some(CString(vec!(4, 3, 2, 1, 0))));
        Result::<(), mpsc::SendError>::Ok(())
    });
    task::block_on(async move { jh.await })?;
    Ok(())
}

// The parser itself needs to keep track of which bytes it has read
// since it produced the last CString. It also needs to be able to
// process the next byte from the underlying stream, returning whether
// or not it has finished the CString or needs more bytes

struct CStringParser {
    context: Vec<u8>,
}

impl CStringParser {
    fn new() -> Self {
        Self {
            context: Vec::new(),
        }
    }

    fn process_next_byte(&mut self, byte: u8) -> ParseStatus<CString> {
        log::debug!("processing byte: {byte}, context = {:?}", self.context);
        self.context.push(byte);
        match self.context.last().unwrap() {
            0 => ParseStatus::Finished(CString(self.context.drain(..).collect())),
            _ => ParseStatus::Incomplete,
        }
    }
}

enum ParseStatus<T> {
    Finished(T),
    Incomplete,
}

// I now need to create a trait that allows streams to call
// the parse_with() method, consuming the stream and returning
// a new stream that produces items of the appropriate type. I also
// need to annotate the fact that mpsc::Receiver satisfies this
// trait.

trait ParseWith<I, O, P>: Stream<Item = I> + Sized
where
    P: Parser<I, O>,
{
    fn parse_with(self, parser: P) -> ParseWithStream<I, O, P, Self> {
        ParseWithStream {
            inner: Mutex::new(self),
            parser: Mutex::new(parser),
            _phantom_data: PhantomData,
        }
    }
}

impl<I, O, P> ParseWith<I, O, P> for mpsc::Receiver<I> where P: Parser<I, O> {}

// This implies the existence of a Parser trait

trait Parser<I, O> {
    fn process_next_item(&mut self, item: I) -> ParseStatus<O>;
}

// Implementation for CStringParser is straightforward

impl Parser<u8, CString> for CStringParser {
    fn process_next_item(&mut self, item: u8) -> ParseStatus<CString> {
        self.process_next_byte(item)
    }
}

// Now I need to create the struct ParseWithStream and implement Stream
// for it. This is where I get stuck trying to understand what the
// type checker is telling me.

struct ParseWithStream<I, O, P, Si>
where
    P: Parser<I, O>,
    Si: Stream<Item = I>,
{
    inner: Mutex<Si>,
    parser: Mutex<P>,
    _phantom_data: PhantomData<O>,
}

/// ```
/// impl<I, O, P, Si> Stream for ParseWithStream<I, O, P, Si>
/// where
///     P: Parser<I, O>,
///     Si: Stream<Item = I>,
/// {
///     type Item = O;
///     fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
///         let next_item: I = match self.inner.poll_next(cx) {
///             Poll::Ready(Some(t)) => t,
///             Poll::Ready(None) => return Poll::Ready(None),
///             Poll::Pending => return Poll::Pending,
///         };
///         match self.parser.process_next_item(next_item) {
///             ParseStatus::Finished(o) => Poll::Ready(Some(o)),
///             ParseStatus::Incomplete => Poll::Pending,
///         }
///     }
/// }
/// ```
///
/// So my first question is,
///
/// > why is the poll_next method is not available, even though `inner`
/// must satisfy `Si: Stream<...>`?
//
/// From [this issue](https://github.com/rust-lang/futures-rs/issues/1513)
/// it looks like I might neet to be using `poll_next_unpin`. This requires
/// changing the trait bounds on `Si` in both the `ParseWithStream` struct
/// and its `Stream` implmentation to be `Unpin`.
///
impl<I, O, P, Si> Stream for ParseWithStream<I, O, P, Si>
where
    I: std::fmt::Display,
    P: Parser<I, O>,
    Si: Stream<Item = I> + Unpin,
{
    type Item = O;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        log::debug!("polling next item");
        let next_item: I = match self.inner.lock().unwrap().poll_next_unpin(cx) {
            Poll::Ready(Some(t)) => t,
            Poll::Ready(None) => return Poll::Ready(None),
            Poll::Pending => return Poll::Pending,
        };
        log::debug!("got next item: {next_item}");
        match self.parser.lock().unwrap().process_next_item(next_item) {
            ParseStatus::Finished(o) => {
                log::debug!("ParseStatus: Finished");
                Poll::Ready(Some(o))
            }
            ParseStatus::Incomplete => {
                log::debug!("ParseStatus: Incomplete");
                Poll::Pending
            }
        }
    }
}

// This results in the type error:
//
// > cannot borrow data in dereference of `Pin<&mut ParseWithStream<I, O, P, Si>>` as mutable
// trait `DerefMut` is required to modify through a dereference,
// but it is not implemented for `Pin<&mut ParseWithStream<I, O, P, Si>>`
//
// So I tried adding `DerefMut` as a constraint to `Si` in both the `ParseWithStream`
// struct, its `Stream` implementation, and my `ParseWith` trait.
//
// I now get

// > the trait bound `futures::futures_channel::mpsc::Receiver<I>: DerefMut`
// is not satisfied
// the trait `DerefMut` is not implemented for
// `futures::futures_channel::mpsc::Receiver<I>`
//
// I tried wrapping `ParseWithStream::inner` in all sorts of permutations of
// `Arc`, `Mutex`, `Pin`, and `Box`, but all my attempts still result in this
// fundmantal problem.
//
