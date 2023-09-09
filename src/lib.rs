#![doc = include_str!("../README.md")]
#![doc = include_str!("example.md")]

use futures_core::future::BoxFuture;
use std::cmp::min;
use std::collections::VecDeque;
use std::fmt;
use std::future::Future;
use std::iter::Peekable;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::task::Context;
use std::task::Poll;
use std::task::Waker;

const UNBOUNDED_CAPACITY: usize = usize::MAX;

// TODO: we could replace Arc with Box and rely on atomic tx_count and
// rx_count.
#[derive(Debug)]
struct State<T> {
    queue: VecDeque<T>,
    capacity: usize,
    tx_count: usize,
    rx_count: usize,
    tx_wakers: Vec<Waker>,
    rx_wakers: Vec<Waker>,
}

fn wake_all_tx<T>(mut state: MutexGuard<State<T>>) {
    let wakers = std::mem::take(&mut state.tx_wakers);
    drop(state);
    for waker in wakers {
        waker.wake();
    }
}

fn wake_all_rx<T>(mut state: MutexGuard<State<T>>) {
    let wakers = std::mem::take(&mut state.rx_wakers);
    drop(state);
    for waker in wakers {
        waker.wake();
    }
}

impl<T> State<T> {
    fn target_capacity(&self) -> usize {
        // TODO: We could offer an option to use queue.capacity
        // instead.
        self.capacity
    }
}

// Sender

/// The sending half of an unbounded channel.
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
            wake_all_rx(state);
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
        wake_all_rx(state);

        Ok(())
    }

    /// Send multiple values.
    ///
    /// If all receivers are dropped, the values are returned in
    /// [SendError] untouched. Either the entire batch is sent or none
    /// of it is sent.
    pub fn send_iter<I>(&self, values: I) -> Result<(), SendError<I>>
    where
        I: IntoIterator<Item = T>,
    {
        let mut state = self.state.lock().unwrap();
        if state.rx_count == 0 {
            assert!(state.queue.is_empty());
            return Err(SendError(values));
        }

        state.queue.extend(values);

        // There is no guarantee that the highest-priority waker will
        // actually call poll() again. Therefore, the best we can do
        // is wake everyone.
        wake_all_rx(state);

        Ok(())
    }

    /// Drain a [Vec] into the channel without deallocating it.
    ///
    /// This is a convenience method for allocation-free batched
    /// sends. The `values` vector is drained, and then returned with
    /// the same capacity it had.
    pub fn send_vec(&self, mut values: Vec<T>) -> Result<Vec<T>, SendError<Vec<T>>> {
        let mut state = self.state.lock().unwrap();
        if state.rx_count == 0 {
            assert!(state.queue.is_empty());
            return Err(SendError(values));
        }

        state.queue.extend(values.drain(..));

        // There is no guarantee that the highest-priority waker will
        // actually call poll() again. Therefore, the best we can do
        // is wake everyone.
        wake_all_rx(state);

        Ok(values)
    }

    /// Converts this [Sender] into a [BatchSender] with the specified
    /// capacity.
    ///
    /// [BatchSender] manages a single allocation containing
    /// `capacity` elements and automatically sends batches as it
    /// fills.
    pub fn batch(self, capacity: usize) -> BatchSender<T> {
        BatchSender {
            sender: self,
            capacity,
            buffer: Vec::with_capacity(capacity),
        }
    }
}

// BatchSender

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
        // If receivers dropped, there's nothing we can do with any
        // held values.
        _ = self.sender.send_vec(std::mem::take(&mut self.buffer));
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
            self.drain()
        } else {
            Ok(())
        }
    }

    /// Buffers multiple values, sending batches as the internal
    /// buffer reaches capacity.
    pub fn send_iter<I: IntoIterator<Item = T>>(&mut self, values: I) -> Result<(), SendError<()>> {
        // TODO: We could return the remainder of I under cancellation.
        for value in values.into_iter() {
            self.send(value)?;
        }
        Ok(())
    }

    /// Sends any buffered values, clearing the current batch.
    pub fn drain(&mut self) -> Result<(), SendError<()>> {
        // TODO: send_iter
        match self.sender.send_vec(std::mem::take(&mut self.buffer)) {
            Ok(drained_vec) => {
                self.buffer = drained_vec;
                Ok(())
            }
            Err(_) => Err(SendError(())),
        }
    }
}

// BoundedSender

/// The sending half of a bounded channel.
#[derive(Debug)]
pub struct BoundedSender<T> {
    state: Arc<Mutex<State<T>>>,
}

impl<T> Drop for BoundedSender<T> {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap();
        assert!(state.tx_count >= 1);
        state.tx_count -= 1;
        if state.tx_count == 0 {
            wake_all_rx(state);
        }
    }
}

impl<T: 'static> BoundedSender<T> {
    /// Send a single value.
    ///
    /// Returns [SendError] if all receivers are dropped.
    pub fn send(&self, value: T) -> impl Future<Output = Result<(), SendError<T>>> + '_ {
        Send {
            sender: self,
            value: Some(value),
        }
    }

    /// Send multiple values.
    ///
    /// If all receivers are dropped, the values are returned in
    /// [SendError] untouched. Either the entire batch is sent or none
    /// of it is sent.
    pub fn send_iter<'a, I>(
        &'a self,
        values: I,
    ) -> impl Future<Output = Result<(), SendError<()>>> + 'a
    where
        I: IntoIterator<Item = T> + 'a,
    {
        SendIter {
            sender: self,
            values: Some(values.into_iter().peekable()),
        }
    }

    /// Automatically accumulate sends into a buffer of size `batch`
    /// and send when full.
    ///
    /// The callback's future must be boxed to work around [type system
    /// limitations in Rust](https://smallcultfollowing.com/babysteps/blog/2023/03/29/thoughts-on-async-closures/).
    ///
    /// TODO: add a feature that gates this only dependency on `futures` crate.
    pub async fn autobatch<F, R>(self, capacity: usize, f: F) -> Result<R, SendError<()>>
    where
        for<'a> F:
            (FnOnce(&'a mut BoundedBatchSender<T>) -> BoxFuture<'a, Result<R, SendError<()>>>),
    {
        let mut tx = BoundedBatchSender {
            sender: self,
            capacity,
            buffer: Vec::with_capacity(capacity),
        };
        let r = f(&mut tx).await?;
        tx.drain().await?;
        Ok(r)
    }
}

#[must_use = "futures do nothing unless you `.await` or poll them"]
struct Send<'a, T> {
    sender: &'a BoundedSender<T>,
    value: Option<T>,
}

impl<'a, T> Future for Send<'a, T> {
    type Output = Result<(), SendError<T>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.sender.state.lock().unwrap();
        if state.rx_count == 0 {
            return Poll::Ready(Err(SendError(self.as_mut().value.take().unwrap())));
        }
        if state.queue.len() < state.target_capacity() {
            state.queue.push_back(self.as_mut().value.take().unwrap());
            wake_all_rx(state);
            Poll::Ready(Ok(()))
        } else {
            state.tx_wakers.push(cx.waker().clone());
            Poll::Pending
        }
    }
}

impl<'a, T> Unpin for Send<'a, T> {}

#[must_use = "futures do nothing unless you `.await` or poll them"]
struct SendIter<'a, T, I: Iterator<Item = T>> {
    sender: &'a BoundedSender<T>,
    values: Option<Peekable<I>>,
}

impl<'a, T, I: Iterator<Item = T>> Future for SendIter<'a, T, I> {
    type Output = Result<(), SendError<()>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.sender.state.lock().unwrap();

        // There is an awkward set of constraints here.
        // 1. To check whether an iterator contains an item, one must be popped.
        // 2. If the receivers are cancelled, we'd like to return the iterator whole.
        // 3. If we don't know whether there are any remaining items, we must block
        //    if the queue is at capacity.
        // We relax constraint #2 because #3 is preferable.
        // TODO: We could return Peekable<I> instead.

        let pi = self.values.as_mut().unwrap();
        loop {
            if pi.peek().is_none() {
                // TODO: We could optimize the case that send_iter was called with an empty
                // iterator, but that's unlikely. We probably sent a message in this loop.
                wake_all_rx(state);
                return Poll::Ready(Ok(()));
            } else if state.rx_count == 0 {
                // TODO: add a test for when receiver is dropped after iterator is drained
                return Poll::Ready(Err(SendError(())));
            } else if state.queue.len() < state.target_capacity() {
                state.queue.push_back(pi.next().unwrap());
            } else {
                state.tx_wakers.push(cx.waker().clone());
                return Poll::Pending;
            }
        }
    }
}

impl<'a, T, I: Iterator<Item = T>> Unpin for SendIter<'a, T, I> {}

// BoundedBatchSender

pub struct BoundedBatchSender<T: 'static> {
    sender: BoundedSender<T>,
    capacity: usize,
    buffer: Vec<T>,
}

impl<T> BoundedBatchSender<T> {
    pub async fn send(&mut self, value: T) -> Result<(), SendError<()>> {
        self.buffer.push(value);
        if self.buffer.len() == self.capacity {
            self.drain().await?;
        }
        Ok(())
    }

    async fn drain(&mut self) -> Result<(), SendError<()>> {
        self.sender.send_iter(self.buffer.drain(..)).await?;
        assert!(self.buffer.is_empty());
        Ok(())
    }
}

// Receiver

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
            wake_all_tx(state);
        }
    }
}

#[must_use = "futures do nothing unless you `.await` or poll them"]
struct Recv<'a, T> {
    receiver: &'a Receiver<T>,
}

impl<'a, T> Unpin for Recv<'a, T> {}

impl<'a, T> Future for Recv<'a, T> {
    type Output = Option<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.receiver.state.lock().unwrap();
        match state.queue.pop_front() {
            Some(value) => {
                wake_all_tx(state);
                Poll::Ready(Some(value))
            }
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
struct RecvBatch<'a, T> {
    receiver: &'a Receiver<T>,
    element_limit: usize,
}

impl<'a, T> Unpin for RecvBatch<'a, T> {}

impl<'a, T> Future for RecvBatch<'a, T> {
    type Output = Vec<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.receiver.state.lock().unwrap();
        let q = &mut state.queue;
        let q_len = q.len();
        if q_len == 0 {
            if state.tx_count == 0 {
                return Poll::Ready(Vec::new());
            } else {
                state.rx_wakers.push(cx.waker().clone());
                return Poll::Pending;
            }
        }

        let capacity = min(q_len, self.element_limit);
        let v = Vec::from_iter(q.drain(..capacity));
        wake_all_tx(state);
        Poll::Ready(v)
    }
}

#[must_use = "futures do nothing unless you .await or poll them"]
struct RecvVec<'a, T> {
    receiver: &'a Receiver<T>,
    element_limit: usize,
    vec: &'a mut Vec<T>,
}

impl<'a, T> Unpin for RecvVec<'a, T> {}

impl<'a, T> Future for RecvVec<'a, T> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.receiver.state.lock().unwrap();
        let q = &mut state.queue;
        let q_len = q.len();
        if q_len == 0 {
            if state.tx_count == 0 {
                assert!(self.vec.is_empty());
                return Poll::Ready(());
            } else {
                state.rx_wakers.push(cx.waker().clone());
                return Poll::Pending;
            }
        }

        let capacity = min(q_len, self.element_limit);
        self.vec.extend(q.drain(..capacity));
        Poll::Ready(())
    }
}

impl<T> Receiver<T> {
    /// Wait for a single value from the channel.
    ///
    /// Returns [None] if all [Sender]s are dropped.
    pub fn recv(&self) -> impl Future<Output = Option<T>> + '_ {
        Recv { receiver: self }
    }

    // TODO: try_recv

    /// Wait for up to `element_limit` values from the channel.
    ///
    /// Up to `element_limit` values are returned if they're already
    /// available. Otherwise, waits for any values to be available.
    ///
    /// Returns an empty [Vec] if all [Sender]s are dropped.
    pub fn recv_batch(&self, element_limit: usize) -> impl Future<Output = Vec<T>> + '_ {
        RecvBatch {
            receiver: self,
            element_limit,
        }
    }

    // TODO: try_recv_batch

    /// Wait for up to `element_limit` values from the channel and
    /// store them in `vec`.
    ///
    /// `vec` should be empty when passed in. Nevertheless, `recv_vec`
    /// will clear it before adding values. The intent of `recv_vec`
    /// is that batches can be repeatedly read by workers without new
    /// allocations.
    ///
    /// It's not required, but `vec`'s capacity should be greater than
    /// or equal to element_limit to avoid reallocation.
    pub fn recv_vec<'a>(
        &'a self,
        element_limit: usize,
        vec: &'a mut Vec<T>,
    ) -> impl Future<Output = ()> + 'a {
        vec.clear();
        RecvVec {
            receiver: self,
            element_limit,
            vec,
        }
    }

    // TODO: try_recv_vec
}

// Constructors

/// Allocates a new, bounded channel and returns the sender, receiver
/// pair.
///
/// Rust async is polling, so synchronous channels are not supported.
/// Therefore, a capacity of 0 is rounded up to 1.
pub fn bounded<T>(capacity: usize) -> (BoundedSender<T>, Receiver<T>) {
    let capacity = capacity.max(1);
    let state = Arc::new(Mutex::new(State {
        queue: VecDeque::new(),
        capacity,
        tx_count: 1,
        rx_count: 1,
        tx_wakers: Vec::new(),
        rx_wakers: Vec::new(),
    }));
    (
        BoundedSender {
            state: state.clone(),
        },
        Receiver { state },
    )
}

/// Allocates a new, unbounded channel and returns the sender,
/// receiver pair.
pub fn unbounded<T>() -> (Sender<T>, Receiver<T>) {
    let state = Arc::new(Mutex::new(State {
        queue: VecDeque::new(),
        capacity: UNBOUNDED_CAPACITY,
        tx_count: 1,
        rx_count: 1,
        tx_wakers: Vec::new(),
        rx_wakers: Vec::new(),
    }));
    (
        Sender {
            state: state.clone(),
        },
        Receiver { state },
    )
}
