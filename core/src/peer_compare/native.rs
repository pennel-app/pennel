use std::{
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf};

use super::{PeerCompareError, wire::Channel};

/// Blocking native operations run only on the dedicated QA blocking pool.
#[uniffi::export(with_foreign)]
pub trait PeerCompareNativeChannel: Send + Sync {
    fn peer_id(&self) -> String;
    fn read(&self, max_bytes: u32) -> Result<Vec<u8>, PeerCompareError>;
    fn write(&self, bytes: Vec<u8>) -> Result<u32, PeerCompareError>;
    fn finish_write(&self) -> Result<(), PeerCompareError>;
    fn close(&self);
}

#[uniffi::export(with_foreign)]
pub trait PeerCompareNativeTransport: Send + Sync {
    fn identity_id(&self) -> String;
    fn address_json(&self) -> Result<String, PeerCompareError>;
    fn connect(
        &self,
        address_json: String,
        expected_peer: String,
    ) -> Result<Arc<dyn PeerCompareNativeChannel>, PeerCompareError>;
    fn accept(&self) -> Result<Arc<dyn PeerCompareNativeChannel>, PeerCompareError>;
    fn approve_peer(&self, id: String) -> Result<bool, PeerCompareError>;
    fn revoke_peer(&self, id: String) -> Result<bool, PeerCompareError>;
    fn close(&self);
}

const NATIVE_BUFFER: usize = 64 * 1024;

struct NativeStream {
    local: DuplexStream,
    channel: Arc<dyn PeerCompareNativeChannel>,
    writer: tokio::sync::oneshot::Receiver<Result<(), PeerCompareError>>,
    writer_done: bool,
}

impl Drop for NativeStream {
    fn drop(&mut self) {
        self.channel.close();
    }
}

impl AsyncRead for NativeStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.local).poll_read(cx, buf)
    }
}
impl AsyncWrite for NativeStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.local).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.local).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.writer_done {
            return Poll::Ready(Ok(()));
        }
        match Pin::new(&mut self.local).poll_shutdown(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
            Poll::Ready(Ok(())) => {}
        }
        match Pin::new(&mut self.writer).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(Ok(()))) => {
                self.writer_done = true;
                Poll::Ready(Ok(()))
            }
            Poll::Ready(_) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "native QA write failed",
            ))),
        }
    }
}

pub(crate) fn stream(channel: Arc<dyn PeerCompareNativeChannel>) -> Channel {
    let (local, bridge) = tokio::io::duplex(NATIVE_BUFFER);
    let (mut reader, mut writer) = tokio::io::split(bridge);
    let native_read = channel.clone();
    tokio::spawn(async move {
        loop {
            let native = native_read.clone();
            let result =
                tokio::task::spawn_blocking(move || native.read(NATIVE_BUFFER as u32)).await;
            let Ok(Ok(bytes)) = result else {
                native_read.close();
                break;
            };
            if bytes.len() > NATIVE_BUFFER {
                native_read.close();
                break;
            }
            if bytes.is_empty() {
                break;
            }
            if writer.write_all(&bytes).await.is_err() {
                native_read.close();
                break;
            }
        }
        // The other split half still owns the duplex stream when a native read fails.
        let _ = writer.shutdown().await;
    });
    let (sent, finished) = tokio::sync::oneshot::channel();
    let native_write = channel.clone();
    tokio::spawn(async move {
        let result = async {
            let mut bytes = vec![0; NATIVE_BUFFER];
            loop {
                let count = reader
                    .read(&mut bytes)
                    .await
                    .map_err(|_| PeerCompareError::Transport)?;
                if count == 0 {
                    let native = native_write.clone();
                    return tokio::task::spawn_blocking(move || native.finish_write())
                        .await
                        .map_err(|_| PeerCompareError::Transport)?;
                }
                let mut offset = 0;
                while offset < count {
                    let owned = bytes[offset..count].to_vec();
                    let native = native_write.clone();
                    let written = tokio::task::spawn_blocking(move || native.write(owned))
                        .await
                        .map_err(|_| PeerCompareError::Transport)??
                        as usize;
                    if written == 0 || written > count - offset {
                        return Err(PeerCompareError::InvalidProtocol);
                    }
                    offset += written;
                }
            }
        }
        .await;
        if result.is_err() {
            native_write.close();
        }
        let _ = sent.send(result);
    });
    Box::new(NativeStream {
        local,
        channel,
        writer: finished,
        writer_done: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peer_compare::{Cancellation, wire};
    use std::sync::Mutex;

    struct ClosedReader {
        chunk: Mutex<Option<Vec<u8>>>,
        buffered: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    }
    impl PeerCompareNativeChannel for ClosedReader {
        fn peer_id(&self) -> String {
            "a".repeat(64)
        }
        fn read(&self, _: u32) -> Result<Vec<u8>, PeerCompareError> {
            if let Some(bytes) = self.chunk.lock().unwrap().take() {
                return Ok(bytes);
            }
            // The pump asks for the next chunk only after it buffered the previous one.
            let _ = self.buffered.lock().unwrap().take().unwrap().send(());
            Err(PeerCompareError::Closed)
        }
        fn write(&self, bytes: Vec<u8>) -> Result<u32, PeerCompareError> {
            Ok(bytes.len() as u32)
        }
        fn finish_write(&self) -> Result<(), PeerCompareError> {
            Ok(())
        }
        fn close(&self) {}
    }

    async fn buffered_stream(bytes: &[u8]) -> Channel {
        let (sent, buffered) = tokio::sync::oneshot::channel();
        let channel = stream(Arc::new(ClosedReader {
            chunk: Mutex::new(Some(bytes.to_vec())),
            buffered: Mutex::new(Some(sent)),
        }));
        tokio::time::timeout(std::time::Duration::from_secs(2), buffered)
            .await
            .expect("the native pump must buffer the fixture before the test")
            .unwrap();
        channel
    }

    #[tokio::test]
    async fn native_read_failure_signals_the_open_duplex_reader() {
        let mut stream = buffered_stream(b"x").await;
        assert_eq!(stream.read_u8().await.unwrap(), b'x');
        let read = tokio::time::timeout(std::time::Duration::from_secs(1), stream.read_u8())
            .await
            .expect("read termination must reach the wire while the write half is open");
        assert_eq!(read.unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
    }

    #[tokio::test]
    async fn cancelled_approval_stops_the_wire_with_native_bytes_already_buffered() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        let stage = dir.path().join("stage");
        let approval = Cancellation::default();
        let stream = buffered_stream(b"ordinary native fixture").await;
        approval.cancel();
        assert_eq!(
            wire::receive(stream, stage.clone(), target.clone(), None, approval).await,
            Err(PeerCompareError::Cancelled)
        );
        assert!(!stage.exists());
        assert!(!target.exists());
    }
}
