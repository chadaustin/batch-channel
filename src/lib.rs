#![doc = include_str!("../README.md")]
#![doc = include_str!("example.md")]

use futures_core::future::BoxFuture;
use pin_project::pin_project;
use std::cmp::min;
use std::collections::VecDeque;
use std::fmt;
use std::future::Future;
use std::iter::Peekable;
use std::pin::Pin;
use std::sync::OnceLock;
use std::task::Context;
use std::task::Poll;
use std::task::Waker;

mod mutex;

use mutex::Condvar;
use mutex::Mutex;
use mutex::MutexGuard;

const UNBOUNDED_CAPACITY: usize = usize::MAX;

macro_rules! derive_clone {
    ($t:ident) => {
        impl<T> Clone for $t<T> {
            fn clone(&self) -> Self {
                Self {
                    core: self.core.clone(),
                }
            }
        }
    };
}

/// Slightly more optimized than Vec<Waker> and avoids an allocation
/// when only one task is blocked.
#[cfg(feature = "smallvec")]
type WakerList = smallvec::SmallVec<[Waker; 1]>;

#[cfg(not(feature = "smallvec"))]
type WakerList = Vec<Waker>;

#[derive(Debug)]
#[pin_project]
struct StateBase {
    capacity: usize,
    closed: bool,
    // An intrusive linked list through the futures would also work
    // and avoid allocation here.
    #[pin]
    tx_wakers: WakerList,
    #[pin]
    rx_wakers: WakerList,
}

impl StateBase {
    fn target_capacity(&self) -> usize {
        // TODO: We could offer an option to use queue.capacity
        // instead.
        self.capacity
    }

    fn pending_tx<T>(&mut self, cx: &mut Context) -> Poll<T> {
        // This may allocate, but only when the sender is about to
        // block, which is already expensive.
        self.tx_wakers.push(cx.waker().clone());
        Poll::Pending
    }

    fn pending_rx<T>(&mut self, cx: &mut Context) -> Poll<T> {
        // This may allocate, but only when the receiver is about to
        // block, which is already expensive.
        self.rx_wakers.push(cx.waker().clone());
        Poll::Pending
    }
}

#[derive(Debug)]
#[pin_project]
struct State<T> {
    #[pin]
    base: StateBase,
    queue: VecDeque<T>,
}

impl<T> State<T> {
    fn has_capacity(&self) -> bool {
        self.queue.len() < self.target_capacity()
    }
}

impl<T> std::ops::Deref for State<T> {
    type Target = StateBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<T> std::ops::DerefMut for State<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}

#[derive(Debug)]
#[pin_project]
struct Core<T> {
    #[pin]
    state: Mutex<State<T>>,
    // OnceLock ensures Core is Sync and Arc<Core> is Send. But it is
    // not strictly necessary, as these condition variables are only
    // accessed while the lock is held. Alas, Rust does not allow
    // Condvar to be stored under the Mutex.
    not_empty: OnceLock<Condvar>,
    not_full: OnceLock<Condvar>,
}

impl<T> Core<T> {
    /// Returns when there is a value or there are no values and all
    /// senders are dropped.
    fn block_until_not_empty(self: Pin<&Self>) -> MutexGuard<'_, State<T>> {
        fn condition<T>(s: &mut State<T>) -> bool {
            !s.closed && s.queue.is_empty()
        }

        let mut state = self.get_ref().state.lock();
        if !condition(&mut *state) {
            return state;
        }
        // Initialize the condvar while the lock is held. Thus, the
        // caller can, while the lock is held, check whether the
        // condvar must be notified.
        let not_empty = self.not_empty.get_or_init(Default::default);
        not_empty.wait_while(state, condition)
    }

    /// Returns when there is either room in the queue or all receivers
    /// are dropped.
    fn block_until_not_full(self: Pin<&Self>) -> MutexGuard<'_, State<T>> {
        fn condition<T>(s: &mut State<T>) -> bool {
            !s.closed && !s.has_capacity()
        }

        let mut state = self.get_ref().state.lock();
        if !condition(&mut *state) {
            return state;
        }
        // Initialize the condvar while the lock is held. Thus, the
        // caller can, while the lock is held, check whether the
        // condvar must be notified.
        let not_full = self.not_full.get_or_init(Default::default);
        not_full.wait_while(state, condition)
    }

    /// Returns when there is either room in the queue or all receivers
    /// are dropped.
    fn wake_rx_and_block_not_full<'a>(
        self: Pin<&Self>,
        mut state: MutexGuard<'a, State<T>>,
    ) -> MutexGuard<'a, State<T>> {
        // The lock is held. Therefore, we know whether a Condvar must
        // be notified or not. Unfortunately, since we aren't dropping
        // the mutex unless the `not_full` condition variable below is not
        // satisfied, we cannot notify `not_empty` outside of the lock.
        if let Some(not_empty) = self.not_empty.get() {
            not_empty.notify_all();
        }

        // TODO: We should not be waking Wakers while a lock is held.
        // I suppose we should release and then reacquire. Reason
        // through this.

        // There is no guarantee that the highest-priority waker will
        // actually call poll() again. Therefore, the best we can do
        // is wake everyone.
        for waker in state.rx_wakers.drain(..) {
            waker.wake();
        }

        // Initialize the condvar while the lock is held. Thus, the
        // caller can, while the lock is held, check whether the
        // condvar must be notified.
        let not_full = self.not_full.get_or_init(Default::default);
        not_full.wait_while(state, |s| !s.closed && !s.has_capacity())
    }

    fn wake_all_tx(self: Pin<&Self>, mut state: MutexGuard<State<T>>) {
        // The lock is held. Therefore, we know whether a Condvar must be notified or not.
        let cvar = self.not_full.get();
        // TODO: keep the tx_wakers allocation somehow
        let wakers = std::mem::take(&mut state.tx_wakers);
        drop(state);
        if let Some(cvar) = cvar {
            // TODO: There are situations where we may know that we can get away with notify_one(), but
            cvar.notify_all();
        }
        // There is no guarantee that the highest-priority waker will
        // actually call poll() again. Therefore, the best we can do
        // is wake everyone.
        for waker in wakers {
            waker.wake();
        }
    }

    fn wake_all_rx(self: Pin<&Self>, mut state: MutexGuard<State<T>>) {
        // The lock is held. Therefore, we know whether a Condvar must be notified or not.
        let cvar = self.not_empty.get();
        // TODO: keep the rx_wakers allocation somehow
        let wakers = std::mem::take(&mut state.rx_wakers);
        drop(state);
        if let Some(cvar) = cvar {
            cvar.notify_all();
        }
        // There is no guarantee that the highest-priority waker will
        // actually call poll() again. Therefore, the best we can do
        // is wake everyone.
        for waker in wakers {
            waker.wake();
        }
    }
}

impl<T> splitrc::Notify for Core<T> {
    fn last_tx_did_drop_pinned(self: Pin<&Self>) {
        let mut state = self.state.lock();
        state.closed = true;
        // We cannot deallocate the queue, as remaining receivers can
        // drain it.
        self.wake_all_rx(state);
    }

    fn last_rx_did_drop_pinned(self: Pin<&Self>) {
        let mut state = self.state.lock();
        state.closed = true;
        // TODO: deallocate
        state.queue.clear();
        self.wake_all_tx(state);
    }
}

// SendError

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

// SyncSender

/// The sending half of a channel.
#[derive(Debug)]
pub struct SyncSender<T> {
    core: Pin<splitrc::Tx<Core<T>>>,
}

derive_clone!(SyncSender);

impl<T> SyncSender<T> {
    /// Converts `SyncSender` to asynchronous `Sender`.
    pub fn into_async(self) -> Sender<T> {
        Sender { core: self.core }
    }

    /// Send a single value.
    ///
    /// Returns [SendError] if all receivers are dropped.
    pub fn send(&self, value: T) -> Result<(), SendError<T>> {
        let mut state = self.core.as_ref().block_until_not_full();
        if state.closed {
            assert!(state.queue.is_empty());
            return Err(SendError(value));
        }

        state.queue.push_back(value);

        self.core.as_ref().wake_all_rx(state);
        Ok(())
    }

    /// Send multiple values.
    ///
    /// If all receivers are dropped, SendError is returned. The
    /// values cannot be returned, as they may have been partially
    /// sent when the channel is closed.
    pub fn send_iter<I>(&self, values: I) -> Result<(), SendError<()>>
    where
        I: IntoIterator<Item = T>,
    {
        let mut values = values.into_iter();

        // If the iterator is empty, we can avoid acquiring the lock.
        let Some(mut value) = values.next() else {
            return Ok(());
        };

        let mut state = self.core.as_ref().block_until_not_full();
        'outer: loop {
            if state.closed {
                // We may have sent some values, but the receivers are
                // all dropped, and that cleared the queue.
                assert!(state.queue.is_empty());
                return Err(SendError(()));
            }

            debug_assert!(state.has_capacity());
            state.queue.push_back(value);
            loop {
                match values.next() {
                    Some(v) => {
                        if state.has_capacity() {
                            state.queue.push_back(v);
                        } else {
                            value = v;
                            // We're about to block, but we know we
                            // sent at least one value, so wake any
                            // waiters.
                            state = self.core.as_ref().wake_rx_and_block_not_full(state);
                            continue 'outer;
                        }
                    }
                    None => {
                        // Done pulling from the iterator and know we
                        // sent at least one value.
                        self.core.as_ref().wake_all_rx(state);
                        return Ok(());
                    }
                }
            }
        }
    }

    /// Drain a [Vec] into the channel without deallocating it.
    ///
    /// This is a convenience method for allocation-free batched
    /// sends. The `values` vector is drained, and then returned with
    /// the same capacity it had.
    pub fn send_vec(&self, mut values: Vec<T>) -> Result<Vec<T>, SendError<Vec<T>>> {
        match self.send_iter(values.drain(..)) {
            Ok(_) => Ok(values),
            Err(_) => Err(SendError(values)),
        }
    }

    /// Automatically accumulate sends into a buffer of size `batch_limit`
    /// and send when full.
    pub fn autobatch<'a, F, R>(&'a mut self, batch_limit: usize, f: F) -> Result<R, SendError<()>>
    where
        F: (FnOnce(&mut SyncBatchSender<'a, T>) -> Result<R, SendError<()>>),
    {
        let mut tx = SyncBatchSender {
            sender: self,
            capacity: batch_limit,
            buffer: Vec::with_capacity(batch_limit),
        };
        let r = f(&mut tx)?;
        tx.drain()?;
        Ok(r)
    }
}

// SyncBatchSender

/// Automatically batches up values and sends them when a batch is full.
#[derive(Debug)]
pub struct SyncBatchSender<'a, T> {
    sender: &'a mut SyncSender<T>,
    capacity: usize,
    buffer: Vec<T>,
}

impl<'a, T> SyncBatchSender<'a, T> {
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

// Sender

/// The asynchronous sending half of a channel.
#[derive(Debug)]
pub struct Sender<T> {
    core: Pin<splitrc::Tx<Core<T>>>,
}

derive_clone!(Sender);

impl<T> Sender<T> {
    /// Converts asynchronous `Sender` to `SyncSender`.
    pub fn into_sync(self) -> SyncSender<T> {
        SyncSender { core: self.core }
    }

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
    /// If all receivers are dropped, SendError is returned and unsent
    /// values are dropped.
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

    /// Automatically accumulate sends into a buffer of size `batch_limit`
    /// and send when full.
    ///
    /// The callback's future must be boxed to work around [type
    /// system limitations in
    /// Rust](https://smallcultfollowing.com/babysteps/blog/2023/03/29/thoughts-on-async-closures/).
    pub async fn autobatch<F, R>(self, batch_limit: usize, f: F) -> Result<R, SendError<()>>
    where
        for<'a> F: (FnOnce(&'a mut BatchSender<T>) -> BoxFuture<'a, Result<R, SendError<()>>>),
    {
        let mut tx = BatchSender {
            sender: self,
            batch_limit,
            buffer: Vec::with_capacity(batch_limit),
        };
        let r = f(&mut tx).await?;
        tx.drain().await?;
        Ok(r)
    }

    /// Same as [Sender::autobatch] except that it immediately returns
    /// () when `f` returns [SendError]. This is a convenience wrapper
    /// for the common case that the future is passed to a spawn
    /// function and the receiver being dropped (i.e. [SendError]) is
    /// considered a clean cancellation.
    pub async fn autobatch_or_cancel<F>(self, capacity: usize, f: F)
    where
        for<'a> F: (FnOnce(&'a mut BatchSender<T>) -> BoxFuture<'a, Result<(), SendError<()>>>),
    {
        self.autobatch(capacity, f).await.unwrap_or(())
    }
}

#[must_use = "futures do nothing unless you `.await` or poll them"]
struct Send<'a, T> {
    sender: &'a Sender<T>,
    value: Option<T>,
}

impl<'a, T> Future for Send<'a, T> {
    type Output = Result<(), SendError<T>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.sender.core.state.lock();
        if state.closed {
            return Poll::Ready(Err(SendError(self.as_mut().value.take().unwrap())));
        }
        if state.has_capacity() {
            state.queue.push_back(self.as_mut().value.take().unwrap());
            self.sender.core.as_ref().wake_all_rx(state);
            Poll::Ready(Ok(()))
        } else {
            state.pending_tx(cx)
        }
    }
}

impl<'a, T> Unpin for Send<'a, T> {}

#[must_use = "futures do nothing unless you `.await` or poll them"]
struct SendIter<'a, T, I: Iterator<Item = T>> {
    sender: &'a Sender<T>,
    values: Option<Peekable<I>>,
}

impl<'a, T, I: Iterator<Item = T>> Future for SendIter<'a, T, I> {
    type Output = Result<(), SendError<()>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Optimize the case that send_iter was called with an empty
        // iterator, and don't even acquire the lock.
        {
            let pi = self.values.as_mut().unwrap();
            if pi.peek().is_none() {
                return Poll::Ready(Ok(()));
            }
            // Satisfy borrow checker: we cannot hold a mut reference to
            // self through pi before acquiring the lock below.
        }

        let mut state = self.sender.core.state.lock();

        // There is an awkward set of constraints here.
        // 1. To check whether an iterator contains an item, one must be popped.
        // 2. If the receivers are cancelled, we'd like to return the iterator whole.
        // 3. If we don't know whether there are any remaining items, we must block
        //    if the queue is at capacity.
        // We relax constraint #2 because #3 is preferable.
        // TODO: We could return Peekable<I> instead.

        let pi = self.values.as_mut().unwrap();
        // We already checked above.
        debug_assert!(pi.peek().is_some());
        if state.closed {
            Poll::Ready(Err(SendError(())))
        } else if !state.has_capacity() {
            // We know we have a value to send, but there is no room.
            state.pending_tx(cx)
        } else {
            debug_assert!(state.has_capacity());
            state.queue.push_back(pi.next().unwrap());
            while state.has_capacity() {
                match pi.next() {
                    Some(value) => {
                        state.queue.push_back(value);
                    }
                    None => {
                        // Done pulling from the iterator and still
                        // have capacity, so we're done.
                        self.sender.core.as_ref().wake_all_rx(state);
                        return Poll::Ready(Ok(()));
                    }
                }
            }
            // We're out of capacity, and might still have items to
            // send. To avoid a round-trip through the scheduler, peek
            // ahead.
            if pi.peek().is_none() {
                self.sender.core.as_ref().wake_all_rx(state);
                return Poll::Ready(Ok(()));
            }

            // Unconditionally returns Poll::Pending
            let pending = state.pending_tx(cx);
            self.sender.core.as_ref().wake_all_rx(state);
            pending
        }
    }
}

impl<'a, T, I: Iterator<Item = T>> Unpin for SendIter<'a, T, I> {}

// BatchSender

/// The internal send handle used by [Sender::autobatch].
/// Builds a buffer of size `batch_limit` and flushes when it's full.
pub struct BatchSender<T> {
    sender: Sender<T>,
    batch_limit: usize,
    buffer: Vec<T>,
}

impl<T> BatchSender<T> {
    /// Adds a value to the internal buffer and flushes it into the
    /// queue when the buffer fills.
    pub async fn send(&mut self, value: T) -> Result<(), SendError<()>> {
        self.buffer.push(value);
        if self.buffer.len() == self.batch_limit {
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

/// The receiving half of a channel. Reads are asynchronous.
#[derive(Debug)]
pub struct Receiver<T> {
    core: Pin<splitrc::Rx<Core<T>>>,
}

derive_clone!(Receiver);

#[must_use = "futures do nothing unless you `.await` or poll them"]
struct Recv<'a, T> {
    receiver: &'a Receiver<T>,
}

impl<'a, T> Unpin for Recv<'a, T> {}

impl<'a, T> Future for Recv<'a, T> {
    type Output = Option<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.receiver.core.state.lock();
        match state.queue.pop_front() {
            Some(value) => {
                self.receiver.core.as_ref().wake_all_tx(state);
                Poll::Ready(Some(value))
            }
            None => {
                if state.closed {
                    Poll::Ready(None)
                } else {
                    state.pending_rx(cx)
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
        let mut state = self.receiver.core.state.lock();
        let q = &mut state.queue;
        let q_len = q.len();
        if q_len == 0 {
            if state.closed {
                return Poll::Ready(Vec::new());
            } else {
                return state.pending_rx(cx);
            }
        }

        let capacity = min(q_len, self.element_limit);
        let v = Vec::from_iter(q.drain(..capacity));
        self.receiver.core.as_ref().wake_all_tx(state);
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
        let mut state = self.receiver.core.state.lock();
        let q = &mut state.queue;
        let q_len = q.len();
        if q_len == 0 {
            if state.closed {
                assert!(self.vec.is_empty());
                return Poll::Ready(());
            } else {
                return state.pending_rx(cx);
            }
        }

        let capacity = min(q_len, self.element_limit);
        self.vec.extend(q.drain(..capacity));
        self.receiver.core.as_ref().wake_all_tx(state);
        Poll::Ready(())
    }
}

impl<T> Receiver<T> {
    /// Converts asynchronous `Receiver` to `SyncReceiver`.
    pub fn into_sync(self) -> SyncReceiver<T> {
        SyncReceiver { core: self.core }
    }

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

// SyncReceiver

/// The synchronous receiving half of a channel.
#[derive(Debug)]
pub struct SyncReceiver<T> {
    core: Pin<splitrc::Rx<Core<T>>>,
}

derive_clone!(SyncReceiver);

impl<T> SyncReceiver<T> {
    /// Converts `SyncReceiver` to asynchronous `Receiver`.
    pub fn into_async(self) -> Receiver<T> {
        Receiver { core: self.core }
    }

    /// Block waiting for a single value from the channel.
    ///
    /// Returns [None] if all [Sender]s are dropped.
    pub fn recv(&self) -> Option<T> {
        let mut state = self.core.as_ref().block_until_not_empty();
        match state.queue.pop_front() {
            Some(value) => {
                self.core.as_ref().wake_all_tx(state);
                Some(value)
            }
            None => {
                assert!(state.closed);
                None
            }
        }
    }

    /// Block waiting for values from the channel.
    ///
    /// Up to `element_limit` values are returned if they're already
    /// available. Otherwise, waits for any values to be available.
    ///
    /// Returns an empty [Vec] if all [Sender]s are dropped.
    pub fn recv_batch(&self, element_limit: usize) -> Vec<T> {
        let mut state = self.core.as_ref().block_until_not_empty();

        let q = &mut state.queue;
        let q_len = q.len();
        if q_len == 0 {
            assert!(state.closed);
            return Vec::new();
        }

        let capacity = min(q_len, element_limit);
        let v = Vec::from_iter(q.drain(..capacity));
        self.core.as_ref().wake_all_tx(state);
        v
    }

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
    pub fn recv_vec(&self, element_limit: usize, vec: &mut Vec<T>) {
        vec.clear();

        let mut state = self.core.as_ref().block_until_not_empty();
        let q = &mut state.queue;
        let q_len = q.len();
        if q_len == 0 {
            assert!(state.closed);
            // The result vector is already cleared.
            return;
        }

        let capacity = min(q_len, element_limit);
        vec.extend(q.drain(..capacity));
        self.core.as_ref().wake_all_tx(state);
    }
}

// Constructors

/// Allocates a bounded channel and returns the sender, receiver
/// pair.
///
/// Rust async is polling, so unbuffered channels are not supported.
/// Therefore, a capacity of 0 is rounded up to 1.
pub fn bounded<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    let capacity = capacity.max(1);
    let core = Core {
        state: Mutex::new(State {
            base: StateBase {
                capacity,
                closed: false,
                tx_wakers: WakerList::new(),
                rx_wakers: WakerList::new(),
            },
            queue: VecDeque::new(),
        }),
        not_empty: OnceLock::new(),
        not_full: OnceLock::new(),
    };
    let (core_tx, core_rx) = splitrc::pin(core);
    (Sender { core: core_tx }, Receiver { core: core_rx })
}

/// Allocates a bounded channel and returns the synchronous handles as
/// a sender, receiver pair.
///
/// Because handles can be converted freely between sync and async,
/// and Rust async is polling, unbuffered channels are not
/// supported. A capacity of 0 is rounded up to 1.
pub fn bounded_sync<T>(capacity: usize) -> (SyncSender<T>, SyncReceiver<T>) {
    let (tx, rx) = bounded(capacity);
    (tx.into_sync(), rx.into_sync())
}

/// Allocates an unbounded channel and returns the sender,
/// receiver pair.
pub fn unbounded<T>() -> (Sender<T>, Receiver<T>) {
    let core = Core {
        state: Mutex::new(State {
            base: StateBase {
                capacity: UNBOUNDED_CAPACITY,
                closed: false,
                tx_wakers: WakerList::new(),
                rx_wakers: WakerList::new(),
            },
            queue: VecDeque::new(),
        }),
        not_empty: OnceLock::new(),
        not_full: OnceLock::new(),
    };
    let (core_tx, core_rx) = splitrc::pin(core);
    (Sender { core: core_tx }, Receiver { core: core_rx })
}

/// Allocates an unbounded channel and returns the synchronous handles
/// as a sender, receiver pair.
pub fn unbounded_sync<T>() -> (SyncSender<T>, SyncReceiver<T>) {
    let (tx, rx) = unbounded();
    (tx.into_sync(), rx.into_sync())
}
