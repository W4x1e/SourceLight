//! One message on a QUIC stream: a JSON header, then an optional body of raw
//! bytes.
//!
//! ```text
//! u32 big-endian   header length
//! ...              header, JSON
//! u64 big-endian   body length (0 for none)
//! ...              body
//! ```
//!
//! The body exists so a thumbnail or a GPX track travels as itself rather than
//! as base64 inside JSON, which would cost a third more bytes and a copy.
//! Both lengths are checked against a limit before anything is allocated, so
//! a peer cannot make the other side reserve gigabytes by lying about a size.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::time::Sleep;

/// The largest header either side accepts. Control messages are a few hundred
/// bytes; a batch of catalogue rows is the biggest header planned.
pub const MAX_HEADER: usize = 16 * 1024 * 1024;

/// The largest body either side accepts. A 1600 px preview is about 200 KB; a
/// phone's ProRAW original can pass 75 MB.
pub const MAX_BODY: u64 = 256 * 1024 * 1024;

/// Why a frame could not be read or written.
#[derive(Debug)]
pub enum FrameError {
    Io(std::io::Error),
    /// A length beyond the limit, or a header that is not the expected JSON.
    Invalid(String),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::Io(e) => write!(f, "{e}"),
            FrameError::Invalid(why) => write!(f, "invalid message: {why}"),
        }
    }
}

impl std::error::Error for FrameError {}

impl From<std::io::Error> for FrameError {
    fn from(e: std::io::Error) -> Self {
        FrameError::Io(e)
    }
}

/// A stream that fails with `TimedOut` once it goes `limit` without moving a
/// byte, rather than waiting for ever on a connection that stopped carrying
/// data. Slow is fine: every byte that moves starts the wait again.
pub struct Watched<S> {
    inner: S,
    limit: Duration,
    deadline: Pin<Box<Sleep>>,
}

impl<S> Watched<S> {
    pub fn new(inner: S, limit: Duration) -> Self {
        Watched {
            inner,
            limit,
            deadline: Box::pin(tokio::time::sleep(limit)),
        }
    }

    pub fn get_mut(&mut self) -> &mut S {
        &mut self.inner
    }

    /// Something moved, or failed: the wait starts again.
    fn moved(&mut self) {
        let at = tokio::time::Instant::now() + self.limit;
        self.deadline.as_mut().reset(at);
    }

    /// Nothing moved this time: `TimedOut` once the limit has passed.
    fn waiting<T>(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<T>> {
        match self.deadline.as_mut().poll(cx) {
            Poll::Ready(()) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the connection stopped carrying data",
            ))),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Watched<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = &mut *self;
        match Pin::new(&mut this.inner).poll_read(cx, buf) {
            Poll::Ready(r) => {
                this.moved();
                Poll::Ready(r)
            }
            Poll::Pending => this.waiting(cx),
        }
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Watched<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = &mut *self;
        match Pin::new(&mut this.inner).poll_write(cx, buf) {
            Poll::Ready(r) => {
                this.moved();
                Poll::Ready(r)
            }
            Poll::Pending => this.waiting(cx),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        match Pin::new(&mut this.inner).poll_flush(cx) {
            Poll::Ready(r) => {
                this.moved();
                Poll::Ready(r)
            }
            Poll::Pending => this.waiting(cx),
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        match Pin::new(&mut this.inner).poll_shutdown(cx) {
            Poll::Ready(r) => {
                this.moved();
                Poll::Ready(r)
            }
            Poll::Pending => this.waiting(cx),
        }
    }
}

/// Writes one frame. The caller finishes the stream.
pub async fn write_frame<W, H>(w: &mut W, header: &H, body: &[u8]) -> Result<(), FrameError>
where
    W: AsyncWrite + Unpin,
    H: Serialize,
{
    let header = serde_json::to_vec(header).map_err(|e| FrameError::Invalid(e.to_string()))?;
    if header.len() > MAX_HEADER {
        return Err(FrameError::Invalid(format!(
            "header of {} bytes is over the limit",
            header.len()
        )));
    }
    if body.len() as u64 > MAX_BODY {
        return Err(FrameError::Invalid(format!(
            "body of {} bytes is over the limit",
            body.len()
        )));
    }
    w.write_all(&(header.len() as u32).to_be_bytes()).await?;
    w.write_all(&header).await?;
    w.write_all(&(body.len() as u64).to_be_bytes()).await?;
    w.write_all(body).await?;
    w.flush().await?;
    Ok(())
}

/// Reads one frame.
pub async fn read_frame<R, H>(r: &mut R) -> Result<(H, Vec<u8>), FrameError>
where
    R: AsyncRead + Unpin,
    H: DeserializeOwned,
{
    let header_len = r.read_u32().await? as usize;
    if header_len > MAX_HEADER {
        return Err(FrameError::Invalid(format!(
            "header of {header_len} bytes is over the limit"
        )));
    }
    let header = read_exactly(r, header_len as u64).await?;
    let header = serde_json::from_slice(&header).map_err(|e| FrameError::Invalid(e.to_string()))?;

    let body_len = r.read_u64().await?;
    if body_len > MAX_BODY {
        return Err(FrameError::Invalid(format!(
            "body of {body_len} bytes is over the limit"
        )));
    }
    let body = read_exactly(r, body_len).await?;
    Ok((header, body))
}

/// Exactly `len` bytes, held only as they arrive: a length within the limit
/// is still only a claim, and allocating it up front let a peer that sent
/// nothing more — before any check that it is paired — hold 256 MB per stream.
async fn read_exactly<R: AsyncRead + Unpin>(r: &mut R, len: u64) -> Result<Vec<u8>, FrameError> {
    let mut out = Vec::with_capacity(len.min(64 * 1024) as usize);
    (&mut *r).take(len).read_to_end(&mut out).await?;
    if (out.len() as u64) < len {
        return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sl_protocol::{Request, Response};

    /// A stream that goes quiet gives up after the limit, not never.
    #[tokio::test(start_paused = true)]
    async fn a_silent_stream_times_out() {
        let (_quiet, them) = tokio::io::duplex(64);
        let mut them = Watched::new(them, Duration::from_secs(30));
        let started = tokio::time::Instant::now();
        match read_frame::<_, Response>(&mut them).await {
            Err(FrameError::Io(e)) => assert_eq!(e.kind(), io::ErrorKind::TimedOut),
            other => panic!("expected a timeout, got {other:?}"),
        }
        assert_eq!(started.elapsed().as_secs(), 30);
    }

    /// Slow but moving is not stalled, however long the whole takes.
    #[tokio::test(start_paused = true)]
    async fn a_slow_stream_that_keeps_moving_is_waited_for() {
        let (mut us, them) = tokio::io::duplex(64);
        let writer = tokio::spawn(async move {
            let mut frame = Vec::new();
            write_frame(&mut frame, &Response::Pong, &[7; 8])
                .await
                .unwrap();
            for byte in frame {
                tokio::time::sleep(Duration::from_secs(20)).await;
                us.write_all(&[byte]).await.unwrap();
            }
        });
        let mut them = Watched::new(them, Duration::from_secs(30));
        let (pong, body): (Response, _) = read_frame(&mut them).await.unwrap();
        assert_eq!((pong, body), (Response::Pong, vec![7; 8]));
        writer.await.unwrap();
    }

    #[tokio::test]
    async fn a_frame_round_trips_with_and_without_a_body() {
        let (mut a, mut b) = tokio::io::duplex(1024);
        let writer = tokio::spawn(async move {
            write_frame(&mut a, &Request::Ping, &[]).await.unwrap();
            write_frame(&mut a, &Response::Pong, &[1, 2, 3])
                .await
                .unwrap();
        });
        let (req, body): (Request, _) = read_frame(&mut b).await.unwrap();
        assert_eq!(req, Request::Ping);
        assert!(body.is_empty());
        let (resp, body): (Response, _) = read_frame(&mut b).await.unwrap();
        assert_eq!(resp, Response::Pong);
        assert_eq!(body, [1, 2, 3]);
        writer.await.unwrap();
    }

    /// A peer that claims a 4 GB header is refused before anything is
    /// allocated for it.
    #[tokio::test]
    async fn an_oversized_length_is_refused_before_allocating() {
        let mut bytes = u32::MAX.to_be_bytes().to_vec();
        bytes.extend_from_slice(b"{}");
        let err = read_frame::<_, Request>(&mut bytes.as_slice())
            .await
            .unwrap_err();
        assert!(matches!(err, FrameError::Invalid(_)), "{err}");

        let header = br#"{"type":"ping"}"#;
        let mut bytes = (header.len() as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(header);
        bytes.extend_from_slice(&u64::MAX.to_be_bytes());
        let err = read_frame::<_, Request>(&mut bytes.as_slice())
            .await
            .unwrap_err();
        assert!(matches!(err, FrameError::Invalid(_)), "{err}");
    }

    #[tokio::test]
    async fn a_cut_off_frame_is_an_error_not_a_hang() {
        let header = br#"{"type":"ping"}"#;
        let mut bytes = (header.len() as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(&header[..5]);
        let err = read_frame::<_, Request>(&mut bytes.as_slice())
            .await
            .unwrap_err();
        assert!(matches!(err, FrameError::Io(_)), "{err}");
    }
}
