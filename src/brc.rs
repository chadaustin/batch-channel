use std::marker::PhantomData;
use std::ops::Deref;
use std::ptr::NonNull;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

pub(crate) trait BrcNotify {
    fn on_tx_drop(&self) {}
    fn on_rx_drop(&self) {}
}

const TX_INC: usize = 2;
const RX_INC: usize = 1;

struct BrcInner<T> {
    // To avoid races between Btx and Brx Drop implementations, the
    // bottom bit of tx_count is set by the deallocating thread.
    tx_count: AtomicUsize,
    rx_count: AtomicUsize,
    data: T,
}

#[derive(Debug)]
pub(crate) struct Btx<T: BrcNotify> {
    ptr: NonNull<BrcInner<T>>,
    phantom: PhantomData<BrcInner<T>>,
}

unsafe impl<T: Sync + Send + BrcNotify> Send for Btx<T> {}
unsafe impl<T: Sync + Send + BrcNotify> Sync for Btx<T> {}

impl<T: BrcNotify> Drop for Btx<T> {
    fn drop(&mut self) {
        // TODO: performance opportunity: if load acquire is 1, no decrement is necessary
        let inner = unsafe { self.ptr.as_ref() };
        if TX_INC == inner.tx_count.fetch_sub(TX_INC, Ordering::AcqRel) {
            inner.data.on_tx_drop();
            if 0 == inner.rx_count.load(Ordering::Acquire) {
                // Both reference counts are observed zero here. But
                // we could be racing with Brx::drop. Use the low bit
                // of tx_count to decide who drops.
                drop(unsafe { Box::from_raw(self.ptr.as_ptr()) });
            }
        }
    }
}

impl<T: BrcNotify> Clone for Btx<T> {
    fn clone(&self) -> Self {
        let inner = unsafe { self.ptr.as_ref() };
        inner.tx_count.fetch_add(TX_INC, Ordering::Relaxed);
        Btx {
            ptr: self.ptr,
            phantom: self.phantom,
        }
    }
}

impl<T: BrcNotify> Deref for Btx<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &unsafe { self.ptr.as_ref() }.data
    }
}

#[derive(Debug)]
pub(crate) struct Brx<T: BrcNotify> {
    ptr: NonNull<BrcInner<T>>,
    phantom: PhantomData<BrcInner<T>>,
}

unsafe impl<T: Sync + Send + BrcNotify> Send for Brx<T> {}
unsafe impl<T: Sync + Send + BrcNotify> Sync for Brx<T> {}

impl<T: BrcNotify> Drop for Brx<T> {
    fn drop(&mut self) {
        // TODO: performance opportunity: if load acquire is 1, no decrement is necessary
        let inner = unsafe { self.ptr.as_ref() };
        if 1 == inner.rx_count.fetch_sub(RX_INC, Ordering::AcqRel) {
            inner.data.on_rx_drop();
            if 0 == inner.tx_count.load(Ordering::Acquire) {
                drop(unsafe { Box::from_raw(self.ptr.as_ptr()) });
            }
        }
    }
}

impl<T: BrcNotify> Clone for Brx<T> {
    fn clone(&self) -> Self {
        let inner = unsafe { self.ptr.as_ref() };
        inner.rx_count.fetch_add(1, Ordering::Relaxed);
        Brx {
            ptr: self.ptr,
            phantom: self.phantom,
        }
    }
}

impl<T: BrcNotify> Deref for Brx<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &unsafe { self.ptr.as_ref() }.data
    }
}

pub(crate) fn new<T: BrcNotify>(data: T) -> (Btx<T>, Brx<T>) {
    let x = Box::new(BrcInner {
        tx_count: AtomicUsize::new(TX_INC),
        rx_count: AtomicUsize::new(RX_INC),
        data,
    });
    let r = Box::leak(x);
    (
        Btx {
            ptr: r.into(),
            phantom: PhantomData,
        },
        Brx {
            ptr: r.into(),
            phantom: PhantomData,
        },
    )
}

#[cfg(test)]
mod tests {
    use crate::brc;
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::Ordering;

    struct Unit;
    impl brc::BrcNotify for Unit {}

    #[derive(Default)]
    struct TrackNotify {
        tx_did_drop: AtomicBool,
        rx_did_drop: AtomicBool,
    }

    impl brc::BrcNotify for TrackNotify {
        fn on_tx_drop(&self) {
            self.tx_did_drop.store(true, Ordering::Release);
        }
        fn on_rx_drop(&self) {
            self.rx_did_drop.store(true, Ordering::Release);
        }
    }

    #[test]
    fn new_and_delete() {
        let (tx, rx) = brc::new(Unit);
        drop(tx);
        drop(rx);
    }

    #[test]
    fn drop_rx_notifies() {
        let (tx, rx) = brc::new(TrackNotify::default());
        let rx2 = rx.clone();
        drop(rx);
        drop(rx2);
        assert!(!tx.tx_did_drop.load(Ordering::Acquire));
        assert!(tx.rx_did_drop.load(Ordering::Acquire));
    }

    #[test]
    fn drop_tx_notifies() {
        let (tx, rx) = brc::new(TrackNotify::default());
        let tx2 = tx.clone();
        drop(tx);
        drop(tx2);
        assert!(rx.tx_did_drop.load(Ordering::Acquire));
        assert!(!rx.rx_did_drop.load(Ordering::Acquire));
    }
}
