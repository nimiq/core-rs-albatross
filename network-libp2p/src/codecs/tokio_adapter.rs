// TODO: Move to nimiq-utils?

use std::{
    io::Error,
    pin::Pin,
    task::{Poll, Context},
};

use futures::io::{AsyncRead, AsyncWrite};
use tokio::io::{AsyncRead as TokioAsyncRead, AsyncWrite as TokioAsyncWrite};
use pin_project::pin_project;

/// An adapter that takes something that is `AsyncRead`/`AsyncWrite` and implements the Tokio
/// counterparts for it.
/// 
#[pin_project]
#[derive(Debug)]
pub struct TokioAdapter<T> {
    #[pin]
    inner: T,
}

impl<T> TokioAdapter<T> {
    pub fn new(inner: T) -> Self {
        Self { inner }
    }

    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T: AsyncRead> AsyncRead for TokioAdapter<T> {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<Result<usize, Error>> {
        AsyncRead::poll_read(self.project().inner, cx, buf)
    }
}

impl<T: AsyncWrite> AsyncWrite for TokioAdapter<T> {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize, Error>> {
        AsyncWrite::poll_write(self.project().inner, cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        AsyncWrite::poll_flush(self.project().inner, cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        AsyncWrite::poll_close(self.project().inner, cx)
    }
}

impl<T: AsyncRead> TokioAsyncRead for TokioAdapter<T> {
    /*    

    Note: I accidentally implemented this adapter for Tokio 1.1. Since we will eventually upgrade
          to this, I'll leave this here :)

    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<Result<(), Error>> {
        // TODO: It's not optimal to initialize the buffer
        match AsyncRead::poll_read(self.project().inner, cx, buf.initialize_unfilled()) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Ok(n_read) => {
                // We read `n_read` bytes and need to advance the filled region of the buffer.
                buf.advance(n_read);
                Ok(())
            }
        }
    }
    */

    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<Result<usize, Error>> {
        AsyncRead::poll_read(self, cx, buf)
    }
}

impl<T: AsyncWrite> TokioAsyncWrite for TokioAdapter<T> {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize, Error>> {
        AsyncWrite::poll_write(self, cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        AsyncWrite::poll_flush(self, cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        AsyncWrite::poll_close(self, cx)
    }
}
