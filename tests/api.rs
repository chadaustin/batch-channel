use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

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

#[derive(Debug)]
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

#[test]
fn closing_rx_drops_elements() {
    let (tx, rx) = batch_channel::bounded_sync(2);
    tx.send(Counter::new()).unwrap();
    assert_eq!(1, COUNT.load(Ordering::Acquire));
    tx.send(Counter::new()).unwrap();
    assert_eq!(2, COUNT.load(Ordering::Acquire));
    drop(rx);
    assert_eq!(0, COUNT.load(Ordering::Acquire));
}
