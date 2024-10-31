use futures::pin_mut;
use std::future::Future;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;
use std::task::Poll;
use std::task::Wake;

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

// cargo test runs threads multithreaded by default. crazy.
static COUNTER_TEST_MUTEX: Mutex<()> = Mutex::new(());

#[test]
fn counter_instances_count() {
    let _l = COUNTER_TEST_MUTEX.lock().unwrap();
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
    let _l = COUNTER_TEST_MUTEX.lock().unwrap();
    let (tx, rx) = batch_channel::bounded_sync(2);
    tx.send(Counter::new()).unwrap();
    assert_eq!(1, COUNT.load(Ordering::Acquire));
    tx.send(Counter::new()).unwrap();
    assert_eq!(2, COUNT.load(Ordering::Acquire));
    drop(rx);
    assert_eq!(0, COUNT.load(Ordering::Acquire));
}

struct TestWake;

impl Wake for TestWake {
    fn wake(self: Arc<Self>) {}
}

#[test]
fn cancel_send() {
    let waker = Arc::new(TestWake).into();
    let mut cx = std::task::Context::from_waker(&waker);

    let (tx, _rx) = batch_channel::bounded(1);
    let send_fut = tx.send('a');
    pin_mut!(send_fut);
    assert_eq!(Poll::Ready(Ok(())), send_fut.poll(&mut cx));

    let send_fut = tx.send('b');
    pin_mut!(send_fut);
    assert_eq!(Poll::Pending, send_fut.as_mut().poll(&mut cx));
    drop(send_fut);
}

#[test]
fn cancel_send_iter() {
    let waker = Arc::new(TestWake).into();
    let mut cx = std::task::Context::from_waker(&waker);

    let (tx, _rx) = batch_channel::bounded(1);
    let send_fut = tx.send_iter(vec!['a', 'b']);
    pin_mut!(send_fut);
    assert_eq!(Poll::Pending, send_fut.as_mut().poll(&mut cx));
    drop(send_fut);
}

#[test]
fn cancel_recv() {
    let waker = Arc::new(TestWake).into();
    let mut cx = std::task::Context::from_waker(&waker);

    let (_tx, rx) = batch_channel::bounded::<char>(1);
    let recv_fut = rx.recv();
    pin_mut!(recv_fut);
    assert_eq!(Poll::Pending, recv_fut.as_mut().poll(&mut cx));
    drop(recv_fut);
}

#[test]
fn cancel_recv_batch() {
    let waker = Arc::new(TestWake).into();
    let mut cx = std::task::Context::from_waker(&waker);

    let (_tx, rx) = batch_channel::bounded::<char>(1);
    let recv_fut = rx.recv_batch(100);
    pin_mut!(recv_fut);
    assert_eq!(Poll::Pending, recv_fut.as_mut().poll(&mut cx));
    drop(recv_fut);
}

#[test]
fn cancel_recv_vec() {
    let waker = Arc::new(TestWake).into();
    let mut cx = std::task::Context::from_waker(&waker);

    let (_tx, rx) = batch_channel::bounded::<char>(1);
    let mut v = Vec::with_capacity(100);
    let recv_fut = rx.recv_vec(v.capacity(), &mut v);
    pin_mut!(recv_fut);
    assert_eq!(Poll::Pending, recv_fut.as_mut().poll(&mut cx));
    drop(recv_fut);
}
