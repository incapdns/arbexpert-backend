//! A multi-producer, single-consumer, futures-aware, FIFO queue.
use super::cell::Cell;
use futures::stream::FusedStream;
use ntex::task::LocalWaker;
use ntex::util::Stream;
use std::collections::VecDeque;
use std::future::poll_fn;
use std::{fmt, panic::UnwindSafe, pin::Pin, task::Context, task::Poll};

/// Creates a unbounded in-memory channel with buffered storage.
pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
  let shared = Cell::new(Shared {
    has_receiver: true,
    buffer: VecDeque::new(),
    blocked_recv: LocalWaker::new(),
  });
  let sender = Sender {
    shared: shared.clone(),
  };
  let receiver = Receiver { shared };
  (sender, receiver)
}

#[derive(Debug)]
struct Shared<T> {
  buffer: VecDeque<T>,
  blocked_recv: LocalWaker,
  has_receiver: bool,
}

/// The transmission end of a channel.
///
/// This is created by the `channel` function.
#[derive(Debug)]
pub struct Sender<T> {
  shared: Cell<Shared<T>>,
}

impl<T> Unpin for Sender<T> {}

impl<T> Sender<T> {
  /// Sends the provided message along this channel.
  pub fn send(&self, item: T) -> Result<(), SendError<T>> {
    let shared = self.shared.get_mut();
    if !shared.has_receiver {
      return Err(SendError(item)); // receiver was dropped
    };
    shared.buffer.push_back(item);
    shared.blocked_recv.wake();
    Ok(())
  }

  /// Closes the sender half
  ///
  /// This prevents any further messages from being sent on the channel while
  /// still enabling the receiver to drain messages that are buffered.
  pub fn close(&self) {
    let shared = self.shared.get_mut();
    shared.has_receiver = false;
    shared.blocked_recv.wake();
  }

  /// Returns whether this channel is closed without needing a context.
  pub fn is_closed(&self) -> bool {
    self.shared.strong_count() == 1 || !self.shared.get_ref().has_receiver
  }
}

impl<T> Clone for Sender<T> {
  fn clone(&self) -> Self {
    Sender {
      shared: self.shared.clone(),
    }
  }
}

impl<T> Drop for Sender<T> {
  fn drop(&mut self) {
    let count = self.shared.strong_count();
    let shared = self.shared.get_mut();

    // check is last sender is about to drop
    if shared.has_receiver && count == 2 {
      // Wake up receiver as its stream has ended
      shared.blocked_recv.wake();
    }
  }
}

/// The receiving end of a channel which implements the `Stream` trait.
///
/// This is created by the `channel` function.
#[derive(Debug)]
pub struct Receiver<T> {
  shared: Cell<Shared<T>>,
}

impl<T> Receiver<T> {
  /// Create a Sender
  pub fn sender(&self) -> Sender<T> {
    Sender {
      shared: self.shared.clone(),
    }
  }

  /// Closes the receiving half of a channel, without dropping it.
  ///
  /// This prevents any further messages from being sent on the channel
  /// while still enabling the receiver to drain messages that are buffered.
  pub fn close(&self) {
    self.shared.get_mut().has_receiver = false;
  }

  /// Returns whether this channel is closed without needing a context.
  pub fn is_closed(&self) -> bool {
    self.shared.strong_count() == 1 || !self.shared.get_ref().has_receiver
  }

  /// Attempt to pull out the next value of this receiver
  /// if the value is available, and returning None if not.
  pub fn try_recv(&self) -> Option<T> {
    let shared = self.shared.get_mut();
    shared.buffer.pop_front()
  }

  /// Attempt to pull out the next value of this receiver, registering
  /// the current task for wakeup if the value is not yet available,
  /// and returning None if the stream is exhausted.
  pub async fn recv(&self) -> Option<T> {
    poll_fn(|cx| self.poll_recv(cx)).await
  }

  /// Attempt to pull out the next value of this receiver, registering
  /// the current task for wakeup if the value is not yet available,
  /// and returning None if the stream is exhausted.
  pub fn poll_recv(&self, cx: &mut Context<'_>) -> Poll<Option<T>> {
    let shared = self.shared.get_mut();

    if let Some(msg) = shared.buffer.pop_front() {
      Poll::Ready(Some(msg))
    } else if shared.has_receiver {
      shared.blocked_recv.register(cx.waker());
      if self.shared.strong_count() == 1 {
        // All senders have been dropped, so drain the buffer and end the
        // stream.
        Poll::Ready(None)
      } else {
        Poll::Pending
      }
    } else {
      Poll::Ready(None)
    }
  }
}

impl<T> Unpin for Receiver<T> {}

impl<T> Stream for Receiver<T> {
  type Item = T;

  fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
    self.poll_recv(cx)
  }
}

impl<T> FusedStream for Receiver<T> {
  fn is_terminated(&self) -> bool {
    self.is_closed()
  }
}

impl<T> UnwindSafe for Receiver<T> {}

impl<T> Drop for Receiver<T> {
  fn drop(&mut self) {
    let shared = self.shared.get_mut();
    shared.buffer.clear();
    shared.has_receiver = false;
  }
}

/// Error type for sending, used when the receiving end of a channel is
/// dropped
pub struct SendError<T>(T);

impl<T> std::error::Error for SendError<T> {}

impl<T> fmt::Debug for SendError<T> {
  fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
    fmt.debug_tuple("SendError").field(&"...").finish()
  }
}

impl<T> fmt::Display for SendError<T> {
  fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(fmt, "send failed because receiver is gone")
  }
}

impl<T> SendError<T> {
  /// Returns the message that was attempted to be sent but failed.
  pub fn into_inner(self) -> T {
    self.0
  }
}