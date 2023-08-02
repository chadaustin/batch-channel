use async_std::task::yield_now;
use async_trait::async_trait;
use futures::executor::LocalPool;
use futures::task::SpawnExt;
use futures::StreamExt;
use std::fmt;
use std::sync::mpsc::TryRecvError;
use std::time::Duration;
use std::time::Instant;

#[async_trait]
trait UnboundedChannel<T> {
    type Sender: Send + 'static;
    type Receiver: Send + 'static;

    fn new() -> (Self::Sender, Self::Receiver);
    fn send(tx: &Self::Sender, value: T) -> anyhow::Result<()>;
    async fn recv(rx: &mut Self::Receiver) -> Option<T>;
}

struct BatchChannel;

#[async_trait]
impl<T: Send + Sync + fmt::Debug + 'static> UnboundedChannel<T> for BatchChannel {
    type Sender = batch_channel::Sender<T>;
    type Receiver = batch_channel::Receiver<T>;

    fn new() -> (Self::Sender, Self::Receiver) {
        batch_channel::unbounded()
    }
    fn send(tx: &Self::Sender, value: T) -> anyhow::Result<()> {
        Ok(tx.send(value)?)
    }
    async fn recv(rx: &mut Self::Receiver) -> Option<T> {
        rx.recv().await
    }
}

struct StdChannel;

#[async_trait]
impl<T: Send + Sync + 'static> UnboundedChannel<T> for StdChannel {
    type Sender = std::sync::mpsc::Sender<T>;
    type Receiver = std::sync::mpsc::Receiver<T>;

    fn new() -> (Self::Sender, Self::Receiver) {
        std::sync::mpsc::channel()
    }
    fn send(tx: &Self::Sender, value: T) -> anyhow::Result<()> {
        Ok(tx.send(value)?)
    }
    async fn recv(rx: &mut Self::Receiver) -> Option<T> {
        loop {
            let r = rx.try_recv();
            match r {
                Ok(value) => return Some(value),
                Err(TryRecvError::Empty) => {
                    yield_now().await;
                    continue;
                }
                Err(TryRecvError::Disconnected) => return None,
            }
        }
    }
}

struct FuturesChannel;

#[async_trait]
impl<T: Send + Sync + 'static> UnboundedChannel<T> for FuturesChannel {
    type Sender = futures::channel::mpsc::UnboundedSender<T>;
    type Receiver = futures::channel::mpsc::UnboundedReceiver<T>;

    fn new() -> (Self::Sender, Self::Receiver) {
        futures::channel::mpsc::unbounded()
    }
    fn send(tx: &Self::Sender, value: T) -> anyhow::Result<()> {
        Ok(tx.unbounded_send(value)?)
    }
    async fn recv(rx: &mut Self::Receiver) -> Option<T> {
        rx.next().await
    }
}

fn single_threaded_one_item_tx_first<UC: UnboundedChannel<usize>>(
    iteration_count: usize,
) -> Duration {
    let mut pool = LocalPool::new();
    let spawner = pool.spawner();

    let (tx, mut rx) = UC::new();

    () = spawner
        .spawn(async move {
            for i in 0..iteration_count {
                UC::send(&tx, i).unwrap();
                // The intent of this benchmark is to interleave send and recv.
                yield_now().await;
            }
        })
        .unwrap();

    () = spawner
        .spawn(async move { while let Some(_) = UC::recv(&mut rx).await {} })
        .unwrap();

    let instant = Instant::now();
    pool.run_until_stalled();

    instant.elapsed()
}

fn single_threaded_one_item_rx_first<UC: UnboundedChannel<usize>>(
    iteration_count: usize,
) -> Duration {
    let mut pool = LocalPool::new();
    let spawner = pool.spawner();

    let (tx, mut rx) = UC::new();

    () = spawner
        .spawn(async move { while let Some(_) = UC::recv(&mut rx).await {} })
        .unwrap();

    () = spawner
        .spawn(async move {
            for i in 0..iteration_count {
                UC::send(&tx, i).unwrap();
                // The intent of this benchmark is to interleave send and recv.
                yield_now().await;
            }
        })
        .unwrap();

    let instant = Instant::now();
    pool.run_until_stalled();

    instant.elapsed()
}

fn bench(name: &str, f: fn(usize) -> Duration) {
    // TODO: disable speedstep
    const N: u32 = 100000;
    print!("{name}... ");
    println!("{:?} per", f(N as usize) / N);
}

fn main() {
    bench(
        "batch_channel tx first 1 item 1 thread",
        single_threaded_one_item_tx_first::<BatchChannel>,
    );
    bench(
        "batch_channel rx first 1 item 1 thread",
        single_threaded_one_item_rx_first::<BatchChannel>,
    );

    bench(
        "futures::channel::mpsc tx first 1 item 1 thread",
        single_threaded_one_item_tx_first::<FuturesChannel>,
    );
    bench(
        "futures::channel::mpsc rx first 1 item 1 thread",
        single_threaded_one_item_rx_first::<FuturesChannel>,
    );

    bench(
        "std::sync::mpsc tx first 1 item 1 thread",
        single_threaded_one_item_tx_first::<StdChannel>,
    );
    bench(
        "std::sync::mpsc rx first 1 item 1 thread",
        single_threaded_one_item_rx_first::<StdChannel>,
    );
}
