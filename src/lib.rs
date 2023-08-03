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

#[derive(Debug, PartialEq, Eq)]
pub struct SendError<T>(pub T);

impl<T> fmt::Display for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "failed to send value on channel")
    }
}

impl<T: fmt::Debug> std::error::Error for SendError<T> {}

impl<T> Sender<T> {
    pub fn batch(self, capacity: usize) -> BatchSender<T> {
        BatchSender {
            sender: self,
            capacity,
            buffer: Vec::with_capacity(capacity),
        }
    }

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
}

pub struct BatchSender<T> {
    sender: Sender<T>,
    capacity: usize,
    buffer: Vec<T>,
}

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

    pub fn send_batch<I: Into<Vec<T>>>(&mut self, values: I) -> Result<(), SendError<()>> {
        for value in values.into() {
            self.send(value)?;
        }
        Ok(())
    }

    // TODO: add a drain method?
}

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
    pub fn recv(&self) -> Recv<'_, T> {
        Recv { receiver: self }
    }

    pub fn recv_batch(&self, element_limit: usize) -> RecvBatch<'_, T> {
        RecvBatch {
            receiver: self,
            element_limit,
        }
    }
}

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
