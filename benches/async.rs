use async_std::task::yield_now;
use async_trait::async_trait;
use futures::executor::LocalPool;
use futures::task::SpawnExt;
use futures::StreamExt;
use std::cmp::max;
use std::fmt;
use std::sync::mpsc::TryRecvError;
use std::time::Duration;
use std::time::Instant;

trait UnboundedChannelName {
    const NAME: &'static str;
}

#[async_trait]
trait UnboundedChannel<T: Send + Sync + 'static>: UnboundedChannelName {
    type Sender: Send + 'static;
    type Receiver: Send + 'static;

    fn new() -> (Self::Sender, Self::Receiver);
    fn send(tx: &Self::Sender, value: T) -> anyhow::Result<()>;
    async fn recv(rx: &mut Self::Receiver) -> Option<T>;

    fn send_many<I: Into<Vec<T>>>(tx: &Self::Sender, values: I) -> anyhow::Result<Vec<T>> {
        let mut values: Vec<_> = values.into();
        for v in values.drain(..) {
            Self::send(tx, v)?;
        }
        Ok(values)
    }

    async fn recv_many(rx: &mut Self::Receiver, element_limit: usize) -> Vec<T> {
        let mut v = Vec::with_capacity(element_limit);
        loop {
            match Self::recv(rx).await {
                Some(value) => {
                    v.push(value);
                    if v.len() == element_limit {
                        return v;
                    }
                }
                None => {
                    return v;
                }
            }
        }
    }
}

struct BatchChannel;

impl UnboundedChannelName for BatchChannel {
    const NAME: &'static str = "batch-channel";
}

#[async_trait]
impl<T: Clone + Send + Sync + fmt::Debug + 'static> UnboundedChannel<T> for BatchChannel {
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
    fn send_many<I: Into<Vec<T>>>(tx: &Self::Sender, values: I) -> anyhow::Result<Vec<T>> {
        Ok(tx.send_many(values)?)
    }
    async fn recv_many(rx: &mut Self::Receiver, element_limit: usize) -> Vec<T> {
        rx.recv_many(element_limit).await
    }
}

struct StdChannel;

impl UnboundedChannelName for StdChannel {
    const NAME: &'static str = "std::sync::mpsc";
}

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

impl UnboundedChannelName for FuturesChannel {
    const NAME: &'static str = "futures::channel::mpsc";
}

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

async fn sender<UC>(tx: UC::Sender, iteration_count: usize, batch_size: usize)
where
    UC: UnboundedChannel<usize>,
{
    if batch_size == 1 {
        for i in 0..iteration_count {
            UC::send(&tx, i).unwrap();
            // The intent of this benchmark is to interleave send and recv.
            yield_now().await;
        }
    } else {
        let mut vec = Vec::with_capacity(batch_size);
        for i in 0..iteration_count {
            if vec.len() < batch_size {
                vec.push(i);
            } else {
                vec = UC::send_many(&tx, vec).unwrap();
                // The intent of this benchmark is to interleave send and recv.
                yield_now().await;
            }
        }
        if !vec.is_empty() {
            _ = UC::send_many(&tx, vec).unwrap();
        }
    }
}

async fn receiver<UC>(mut rx: UC::Receiver, batch_size: usize)
where
    UC: UnboundedChannel<usize> + Send,
{
    if batch_size == 1 {
        while let Some(_) = UC::recv(&mut rx).await {}
    } else {
        loop {
            let v = UC::recv_many(&mut rx, batch_size).await;
            if v.is_empty() {
                break;
            }
        }
    }
}

fn single_threaded_one_item_tx_first<UC>(iteration_count: usize, batch_size: usize) -> Duration
where
    UC: UnboundedChannel<usize> + Send + 'static,
{
    let mut pool = LocalPool::new();
    let spawner = pool.spawner();

    let (tx, rx) = UC::new();

    () = spawner
        .spawn(sender::<UC>(tx, iteration_count, batch_size))
        .unwrap();

    () = spawner.spawn(receiver::<UC>(rx, batch_size)).unwrap();

    let instant = Instant::now();
    pool.run_until_stalled();

    instant.elapsed()
}

fn single_threaded_one_item_rx_first<UC>(iteration_count: usize, batch_size: usize) -> Duration
where
    UC: UnboundedChannel<usize> + Send + 'static,
{
    let mut pool = LocalPool::new();
    let spawner = pool.spawner();

    let (tx, rx) = UC::new();

    () = spawner.spawn(receiver::<UC>(rx, batch_size)).unwrap();

    () = spawner
        .spawn(sender::<UC>(tx, iteration_count, batch_size))
        .unwrap();

    let instant = Instant::now();
    pool.run_until_stalled();

    instant.elapsed()
}

fn bench<Name, F>(name: Name, f: F)
where
    Name: fmt::Display,
    F: FnOnce(usize) -> Duration,
{
    // TODO: disable dynamic frequency scaling
    const N: u32 = 100000;
    print!("{name}... ");
    println!("{:?} per", f(N as usize) / N);
}

fn bench_batch_size<UC>(batch_size: usize)
where
    UC: UnboundedChannel<usize> + Send + 'static,
{
    bench(
        format!(
            "{:width$} tx first 1thr batch={}",
            UC::NAME,
            batch_size,
            width = max_name_len(),
        ),
        |ic| single_threaded_one_item_tx_first::<UC>(ic, batch_size),
    );
    bench(
        format!(
            "{:width$} rx first 1thr batch={}",
            UC::NAME,
            batch_size,
            width = max_name_len(),
        ),
        |ic| single_threaded_one_item_rx_first::<UC>(ic, batch_size),
    );
}

fn max_name_len() -> usize {
    max(
        BatchChannel::NAME.len(),
        max(FuturesChannel::NAME.len(), StdChannel::NAME.len()),
    )
}

fn main() {
    for batch in [1, 10, 100] {
        bench_batch_size::<BatchChannel>(batch);
        bench_batch_size::<FuturesChannel>(batch);
        bench_batch_size::<StdChannel>(batch);
    }
}
