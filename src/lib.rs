#![doc = include_str!("../README.md")]

use std::collections::VecDeque;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::task::Context;
use std::task::Poll;
use std::task::Waker;

// TODO: we could replace Arc with Box and rely on atomic tx_count and
// rx_count.
#[derive(Debug)]
struct State<T> {
    queue: VecDeque<T>,
    tx_count: usize,
    rx_count: usize,
    rx_wakers: Vec<Waker>,
}

fn wake_all<T>(mut state: MutexGuard<State<T>>) {
    let wakers = std::mem::take(&mut state.rx_wakers);
    drop(state);
    for waker in wakers {
        waker.wake();
    }
}

/// The sending half of a channel.
#[derive(Debug)]
pub struct Sender<T> {
    state: Arc<Mutex<State<T>>>,
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        self.state.lock().unwrap().tx_count += 1;
        Sender {
            state: self.state.clone(),
        }
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap();
        assert!(state.tx_count >= 1);
        state.tx_count -= 1;
        if state.tx_count == 0 {
            wake_all(state);
        }
    }
}

/// An error returned from [Sender::send] when all [Receiver]s are
/// dropped.
///
/// The unsent value is returned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SendError<T>(pub T);

impl<T> fmt::Display for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "failed to send value on channel")
    }
}

impl<T: fmt::Debug> std::error::Error for SendError<T> {}

impl<T> Sender<T> {
    /// Send a single value.
    ///
    /// Returns [SendError] if all receivers are dropped.
    pub fn send(&self, value: T) -> Result<(), SendError<T>> {
        let mut state = self.state.lock().unwrap();
        if state.rx_count == 0 {
            assert!(state.queue.is_empty());
            return Err(SendError(value));
        }

        state.queue.push_back(value);

        // There is no guarantee that the highest-priority waker will
        // actually call poll() again. Therefore, the best we can do
        // is wake everyone.
        wake_all(state);

        Ok(())
    }

    /// Send multiple values.
    ///
    /// If all receivers are dropped, the values are returned in
    /// [SendError] untouched. Either the entire batch is sent or none
    /// of it is sent.
    pub fn send_batch<I: Into<Vec<T>>>(&self, values: I) -> Result<Vec<T>, SendError<Vec<T>>> {
        // This iterator might be expensive. Evaluate it before the lock is held.
        let mut values: Vec<_> = values.into();

        let mut state = self.state.lock().unwrap();
        if state.rx_count == 0 {
            assert!(state.queue.is_empty());
            return Err(SendError(values));
        }

        state.queue.extend(values.drain(..));

        // There is no guarantee that the highest-priority waker will
        // actually call poll() again. Therefore, the best we can do
        // is wake everyone.
        wake_all(state);

        Ok(values)
    }

    /// Starts a [BatchSender] with the specified capacity.
    ///
    /// [BatchSender] manages a single allocation containing
    /// `capacity` elements and automatically sends batches when it
    /// reaches capacity.
    pub fn batch(self, capacity: usize) -> BatchSender<T> {
        BatchSender {
            sender: self,
            capacity,
            buffer: Vec::with_capacity(capacity),
        }
    }
}

/// Automatically sends values on the channel in batches.
///
/// Any unsent values are sent upon drop.
#[derive(Debug)]
pub struct BatchSender<T> {
    sender: Sender<T>,
    capacity: usize,
    buffer: Vec<T>,
}

/// Sends remaining values.
impl<T> Drop for BatchSender<T> {
    fn drop(&mut self) {
        if self.buffer.is_empty() {
            return;
        }
        // Nothing to do if receiver dropped.
        _ = self.sender.send_batch(std::mem::take(&mut self.buffer));
    }
}

impl<T> BatchSender<T> {
    /// Buffers a single value to be sent on the channel.
    ///
    /// Sends the batch if the buffer is full.
    pub fn send(&mut self, value: T) -> Result<(), SendError<()>> {
        self.buffer.push(value);
        // TODO: consider using the full capacity if Vec overallocated.
        if self.buffer.len() == self.capacity {
            match self.sender.send_batch(std::mem::take(&mut self.buffer)) {
                Ok(drained_vec) => {
                    self.buffer = drained_vec;
                }
                Err(_) => {
                    return Err(SendError(()));
                }
            }
        }
        Ok(())
    }

    /// Buffers multiple values, sending batches as the internal
    /// buffer reaches capacity.
    pub fn send_batch<I: Into<Vec<T>>>(&mut self, values: I) -> Result<(), SendError<()>> {
        for value in values.into() {
            self.send(value)?;
        }
        Ok(())
    }

    // TODO: add a drain method?
}

/// The receiving half of a channel.
#[derive(Debug)]
pub struct Receiver<T> {
    state: Arc<Mutex<State<T>>>,
}

impl<T> Clone for Receiver<T> {
    fn clone(&self) -> Self {
        self.state.lock().unwrap().rx_count += 1;
        Receiver {
            state: self.state.clone(),
        }
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap();
        assert!(state.rx_count >= 1);
        state.rx_count -= 1;
        if state.rx_count == 0 {
            state.queue.clear();
        }
    }
}

/// A [Future] type that waits for a single value.
///
/// Returns None if all senders are dropped.
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct Recv<'a, T> {
    receiver: &'a Receiver<T>,
}

impl<'a, T> Unpin for Recv<'a, T> {}

impl<'a, T> Future for Recv<'a, T> {
    type Output = Option<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.receiver.state.lock().unwrap();
        match state.queue.pop_front() {
            Some(value) => Poll::Ready(Some(value)),
            None => {
                if state.tx_count == 0 {
                    Poll::Ready(None)
                } else {
                    state.rx_wakers.push(cx.waker().clone());
                    Poll::Pending
                }
            }
        }
    }
}

/// A [Future] type that reads up to `element_limit` values from the
/// queue.
///
/// Up to `element_limit` values are returned if they're already
/// available. Otherwise, waits for any values to be available.
///
/// Returns an empty [Vec] if all senders are dropped.
#[must_use = "futures do nothing unless you .await or poll them"]
pub struct RecvBatch<'a, T> {
    receiver: &'a Receiver<T>,
    element_limit: usize,
}

impl<'a, T> Unpin for RecvBatch<'a, T> {}

impl<'a, T> Future for RecvBatch<'a, T> {
    type Output = Vec<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.receiver.state.lock().unwrap();
        let q = &mut state.queue;
        if q.is_empty() {
            if state.tx_count == 0 {
                Poll::Ready(Vec::new())
            } else {
                state.rx_wakers.push(cx.waker().clone());
                Poll::Pending
            }
        } else if q.len() <= self.element_limit {
            Poll::Ready(Vec::from(std::mem::take(q)))
        } else {
            let drain = q.drain(..self.element_limit);
            Poll::Ready(drain.collect())
        }
    }
}

impl<T> Receiver<T> {
    /// Wait for a single value from the stream.
    ///
    /// Returns [None] if all [Sender]s are dropped.
    pub fn recv(&self) -> Recv<'_, T> {
        Recv { receiver: self }
    }

    /// Wait for up to `element_limit` values from the stream.
    ///
    /// Returns an empty [Vec] if all [Sender]s are dropped.
    pub fn recv_batch(&self, element_limit: usize) -> RecvBatch<'_, T> {
        RecvBatch {
            receiver: self,
            element_limit,
        }
    }
}

/// Allocates a new, unbounded channel and returns the sender,
/// receiver pair.
pub fn unbounded<T>() -> (Sender<T>, Receiver<T>) {
    let state = Arc::new(Mutex::new(State {
        queue: VecDeque::new(),
        tx_count: 1,
        rx_count: 1,
        rx_wakers: Vec::new(),
    }));
    (
        Sender {
            state: state.clone(),
        },
        Receiver { state },
    )
}
