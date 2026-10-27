//! An L2CAP channel's NSInputStream and NSOutputStream as a tokio stream.
//! A thread per channel schedules both streams on its run loop and moves
//! bytes between them and two bounded buffers that the async side reads
//! and writes.

use std::{
    collections::VecDeque,
    io,
    pin::Pin,
    ptr::NonNull,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
    time::Duration,
};

use objc2_core_bluetooth::CBL2CAPChannel;
use objc2_foundation::{
    NSDate, NSDefaultRunLoopMode, NSInputStream, NSOutputStream, NSRunLoop, NSStream,
    NSStreamStatus,
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use super::Shared;

/// Bytes buffered in each direction before the faster side is held back.
const BUFFER: usize = 65536;
/// How long the channel thread lets the run loop run per turn; the buffers
/// are served between turns.
const TICK: Duration = Duration::from_millis(10);

#[derive(Default)]
struct Inner {
    incoming: VecDeque<u8>,
    outgoing: VecDeque<u8>,
    read_waker: Option<Waker>,
    write_waker: Option<Waker>,
    /// The input stream ended or either stream failed.
    peer_closed: bool,
    error: Option<String>,
    /// The async side wants the channel closed once `outgoing` is delivered.
    shutdown: bool,
    /// The async side is gone: close now.
    dropped: bool,
}

impl Inner {
    fn wake_all(&mut self) {
        if let Some(w) = self.read_waker.take() {
            w.wake();
        }
        if let Some(w) = self.write_waker.take() {
            w.wake();
        }
    }
}

/// One L2CAP channel as an async byte stream. Dropping it closes the
/// channel; `shutdown` closes it after what was written has been sent.
pub struct L2capStream {
    state: Arc<Mutex<Inner>>,
}

impl L2capStream {
    /// Takes over the channel's streams on a thread of their own.
    pub fn spawn(channel: Shared<CBL2CAPChannel>) -> io::Result<Self> {
        // SAFETY: no preconditions.
        let input = unsafe { channel.inputStream() }
            .ok_or_else(|| io::Error::other("channel has no input stream"))?;
        let output = unsafe { channel.outputStream() }
            .ok_or_else(|| io::Error::other("channel has no output stream"))?;
        let (input, output) = (Shared(input), Shared(output));
        let state = Arc::new(Mutex::new(Inner::default()));
        let thread_state = state.clone();
        std::thread::Builder::new()
            .name("blessh-l2cap".into())
            .spawn(move || serve(channel, input, output, thread_state))?;
        Ok(Self { state })
    }
}

impl Drop for L2capStream {
    fn drop(&mut self) {
        self.state.lock().unwrap().dropped = true;
    }
}

fn stream_error(stream: &NSStream) -> String {
    stream
        .streamError()
        .map(|e| e.localizedDescription().to_string())
        .unwrap_or_else(|| "stream error".into())
}

fn serve(
    _channel: Shared<CBL2CAPChannel>,
    input: Shared<NSInputStream>,
    output: Shared<NSOutputStream>,
    state: Arc<Mutex<Inner>>,
) {
    let run_loop = NSRunLoop::currentRunLoop();
    // SAFETY: a Foundation constant.
    let mode = unsafe { NSDefaultRunLoopMode };
    // SAFETY: the streams belong to this thread from here on.
    unsafe {
        input.scheduleInRunLoop_forMode(&run_loop, mode);
        output.scheduleInRunLoop_forMode(&run_loop, mode);
    }
    input.open();
    output.open();
    let mut buf = vec![0u8; 4096];
    loop {
        run_loop.runMode_beforeDate(
            mode,
            &NSDate::dateWithTimeIntervalSinceNow(TICK.as_secs_f64()),
        );
        let mut s = state.lock().unwrap();
        if s.dropped {
            break;
        }
        // Deliver what the peer sent, as long as there is room.
        while s.incoming.len() < BUFFER && input.hasBytesAvailable() {
            // SAFETY: the buffer is valid for its length.
            let n =
                unsafe { input.read_maxLength(NonNull::new(buf.as_mut_ptr()).unwrap(), buf.len()) };
            if n > 0 {
                s.incoming.extend(&buf[..n as usize]);
            } else {
                if n < 0 {
                    s.error = Some(stream_error(&input));
                }
                s.peer_closed = true;
                break;
            }
        }
        // Send what the async side queued, as long as the stream takes it.
        while !s.outgoing.is_empty() && !s.peer_closed && output.hasSpaceAvailable() {
            let (head, _) = s.outgoing.as_slices();
            // SAFETY: the slice is valid for its length; the stream only reads it.
            let n = unsafe {
                output.write_maxLength(NonNull::new(head.as_ptr().cast_mut()).unwrap(), head.len())
            };
            if n > 0 {
                s.outgoing.drain(..n as usize);
            } else {
                s.error = Some(stream_error(&output));
                s.peer_closed = true;
            }
        }
        let status = input.streamStatus();
        if status == NSStreamStatus::AtEnd
            || status == NSStreamStatus::Error
            || status == NSStreamStatus::Closed
            || output.streamStatus() == NSStreamStatus::Error
        {
            if status == NSStreamStatus::Error && s.error.is_none() {
                s.error = Some(stream_error(&input));
            }
            s.peer_closed = true;
        }
        s.wake_all();
        if s.peer_closed || (s.shutdown && s.outgoing.is_empty()) {
            break;
        }
    }
    log::debug!(
        "L2CAP channel closing: input {:?}, output {:?}, error {:?}, {} bytes not sent",
        input.streamStatus(),
        output.streamStatus(),
        state.lock().unwrap().error,
        state.lock().unwrap().outgoing.len()
    );
    input.close();
    output.close();
    let mut s = state.lock().unwrap();
    s.peer_closed = true;
    s.wake_all();
}

impl AsyncRead for L2capStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let mut s = self.state.lock().unwrap();
        if !s.incoming.is_empty() {
            let n = buf.remaining().min(s.incoming.len());
            let (a, b) = s.incoming.as_slices();
            let from_a = n.min(a.len());
            buf.put_slice(&a[..from_a]);
            buf.put_slice(&b[..n - from_a]);
            s.incoming.drain(..n);
            return Poll::Ready(Ok(()));
        }
        if s.peer_closed {
            return Poll::Ready(match &s.error {
                Some(e) => Err(io::Error::new(io::ErrorKind::ConnectionReset, e.clone())),
                None => Ok(()),
            });
        }
        s.read_waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

impl AsyncWrite for L2capStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        let mut s = self.state.lock().unwrap();
        if s.peer_closed || s.shutdown {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        }
        let room = BUFFER - s.outgoing.len();
        if room == 0 {
            s.write_waker = Some(cx.waker().clone());
            return Poll::Pending;
        }
        let n = room.min(data.len());
        s.outgoing.extend(&data[..n]);
        Poll::Ready(Ok(n))
    }

    /// Resolves once everything written has been handed to the channel.
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let mut s = self.state.lock().unwrap();
        if s.outgoing.is_empty() || s.peer_closed {
            return Poll::Ready(Ok(()));
        }
        s.write_waker = Some(cx.waker().clone());
        Poll::Pending
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let mut s = self.state.lock().unwrap();
        s.shutdown = true;
        if s.outgoing.is_empty() || s.peer_closed {
            return Poll::Ready(Ok(()));
        }
        s.write_waker = Some(cx.waker().clone());
        Poll::Pending
    }
}
