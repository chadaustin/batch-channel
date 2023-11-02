use std::marker::PhantomData;
use std::ops::Deref;
use std::ptr::NonNull;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

pub(crate) trait BrcNotify {
    fn on_tx_drop(&self) {}
    fn on_rx_drop(&self) {}
}

/// The strategy here is to encode two 32-bit reference counts in a
/// single atomic 64-bit. 32 bits should be suitable for reasonable
/// use. To avoid potential overflow and thus reference count
/// corruption, we detect overflow and panic.
const TX_INC: u64 = 1 << 32;
const RX_INC: u64 = 1;
const RC_INIT: u64 = TX_INC + RX_INC;

fn tx_count(c: u64) -> u32 {
    (c >> 32) as _
}

fn rx_count(c: u64) -> u32 {
    c as _
}

struct BrcInner<T> {
    count: AtomicU64,
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
        let inner = unsafe { self.ptr.as_ref() };
        let old = inner.count.fetch_sub(TX_INC, Ordering::AcqRel);
        if tx_count(old) == 1 {
            inner.data.on_tx_drop();
            if rx_count(old) == 0 {
                // We brought the reference count to zero, so deallocate.
                drop(unsafe { Box::from_raw(self.ptr.as_ptr()) });
            }
        }
    }
}

impl<T: BrcNotify> Clone for Btx<T> {
    fn clone(&self) -> Self {
        let inner = unsafe { self.ptr.as_ref() };
        inner.count.fetch_add(TX_INC, Ordering::Relaxed);
        // TODO: detect saturation and panic
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
        let inner = unsafe { self.ptr.as_ref() };
        let old = inner.count.fetch_sub(RX_INC, Ordering::AcqRel);
        if rx_count(old) == 1 {
            inner.data.on_rx_drop();
            if tx_count(old) == 0 {
                // We brought the reference count to zero, so deallocate.
                drop(unsafe { Box::from_raw(self.ptr.as_ptr()) });
            }
        }
    }
}

impl<T: BrcNotify> Clone for Brx<T> {
    fn clone(&self) -> Self {
        let inner = unsafe { self.ptr.as_ref() };
        inner.count.fetch_add(RX_INC, Ordering::Relaxed);
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
        count: AtomicU64::new(RC_INIT),
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
