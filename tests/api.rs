use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn sync_to_async_and_back() {
    let (tx, rx) = batch_channel::bounded::<()>(1);

    let tx = tx.into_sync();
    let rx = rx.into_sync();

    let tx = tx.into_async();
    let rx = rx.into_async();

    _ = (tx, rx);
}

static COUNT: AtomicUsize = AtomicUsize::new(0);

struct Counter {
    inc: usize,
}

impl Counter {
    fn new() -> Counter {
        let inc = 1;
        COUNT.fetch_add(inc, Ordering::AcqRel);
        Counter { inc }
    }
}

impl Drop for Counter {
    fn drop(&mut self) {
        COUNT.fetch_sub(self.inc, Ordering::AcqRel);
    }
}

#[test]
fn counter_instances_count() {
    assert_eq!(0, COUNT.load(Ordering::Acquire));
    let c1 = Counter::new();
    assert_eq!(1, COUNT.load(Ordering::Acquire));
    let c2 = Counter::new();
    assert_eq!(2, COUNT.load(Ordering::Acquire));
    drop(c1);
    assert_eq!(1, COUNT.load(Ordering::Acquire));
    drop(c2);
    assert_eq!(0, COUNT.load(Ordering::Acquire));
}
