use async_std::task::yield_now;
use async_trait::async_trait;
use futures::executor::LocalPool;
use futures::task::SpawnExt;
use futures::StreamExt;
use std::fmt;
use std::sync::mpsc::TryRecvError;

trait UnboundedChannelName {
    const NAME: &'static str;
}

#[async_trait]
trait UnboundedChannel: UnboundedChannelName {
    type Sender<T: Send + 'static>: Send + 'static;
    type Receiver<T: Send + 'static>: Send + 'static;

    fn new<T: Send>() -> (Self::Sender<T>, Self::Receiver<T>);
    fn send<T: fmt::Debug + Send + Sync + 'static>(
        tx: &Self::Sender<T>,
        value: T,
    ) -> anyhow::Result<()>;
    async fn recv<T: Send>(rx: &mut Self::Receiver<T>) -> Option<T>;

    fn send_vec<T: fmt::Debug + Send + Sync>(
        tx: &Self::Sender<T>,
        mut values: Vec<T>,
    ) -> anyhow::Result<Vec<T>> {
        for v in values.drain(..) {
            Self::send(tx, v)?;
        }
        Ok(values)
    }

    async fn recv_batch<T: Send>(rx: &mut Self::Receiver<T>, element_limit: usize) -> Vec<T> {
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
impl UnboundedChannel for BatchChannel {
    type Sender<T: Send + 'static> = batch_channel::Sender<T>;
    type Receiver<T: Send + 'static> = batch_channel::Receiver<T>;

    fn new<T: Send + 'static>() -> (Self::Sender<T>, Self::Receiver<T>) {
        batch_channel::unbounded()
    }
    fn send<T: fmt::Debug + Send + Sync + 'static>(
        tx: &Self::Sender<T>,
        value: T,
    ) -> anyhow::Result<()> {
        Ok(tx.send(value)?)
    }
    async fn recv<T: Send + 'static>(rx: &mut Self::Receiver<T>) -> Option<T> {
        rx.recv().await
    }
    fn send_vec<T: fmt::Debug + Send + Sync + 'static>(
        tx: &Self::Sender<T>,
        values: Vec<T>,
    ) -> anyhow::Result<Vec<T>> {
        Ok(tx.send_vec(values)?)
    }
    async fn recv_batch<T: Send + 'static>(
        rx: &mut Self::Receiver<T>,
        element_limit: usize,
    ) -> Vec<T> {
        rx.recv_batch(element_limit).await
    }
}

struct StdChannel;

impl UnboundedChannelName for StdChannel {
    const NAME: &'static str = "std::sync::mpsc";
}

#[async_trait]
impl UnboundedChannel for StdChannel {
    type Sender<T: Send + 'static> = std::sync::mpsc::Sender<T>;
    type Receiver<T: Send + 'static> = std::sync::mpsc::Receiver<T>;

    fn new<T: Send + 'static>() -> (Self::Sender<T>, Self::Receiver<T>) {
        std::sync::mpsc::channel()
    }
    fn send<T: fmt::Debug + Send + Sync + 'static>(
        tx: &Self::Sender<T>,
        value: T,
    ) -> anyhow::Result<()> {
        Ok(tx.send(value)?)
    }
    async fn recv<T: Send + 'static>(rx: &mut Self::Receiver<T>) -> Option<T> {
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

struct CrossbeamChannel;

impl UnboundedChannelName for CrossbeamChannel {
    const NAME: &'static str = "crossbeam::channel";
}

#[async_trait]
impl UnboundedChannel for CrossbeamChannel {
    type Sender<T: Send + 'static> = crossbeam::channel::Sender<T>;
    type Receiver<T: Send + 'static> = crossbeam::channel::Receiver<T>;

    fn new<T: Send + 'static>() -> (Self::Sender<T>, Self::Receiver<T>) {
        crossbeam::channel::unbounded()
    }
    fn send<T: fmt::Debug + Send + Sync + 'static>(
        tx: &Self::Sender<T>,
        value: T,
    ) -> anyhow::Result<()> {
        Ok(tx.send(value)?)
    }
    async fn recv<T: Send + 'static>(rx: &mut Self::Receiver<T>) -> Option<T> {
        loop {
            let r = rx.try_recv();
            match r {
                Ok(value) => return Some(value),
                Err(crossbeam::channel::TryRecvError::Empty) => {
                    yield_now().await;
                    continue;
                }
                Err(crossbeam::channel::TryRecvError::Disconnected) => return None,
            }
        }
    }
}

struct FuturesChannel;

impl UnboundedChannelName for FuturesChannel {
    const NAME: &'static str = "futures::channel::mpsc";
}

#[async_trait]
impl UnboundedChannel for FuturesChannel {
    type Sender<T: Send + 'static> = futures::channel::mpsc::UnboundedSender<T>;
    type Receiver<T: Send + 'static> = futures::channel::mpsc::UnboundedReceiver<T>;

    fn new<T: Send + 'static>() -> (Self::Sender<T>, Self::Receiver<T>) {
        futures::channel::mpsc::unbounded()
    }
    fn send<T: fmt::Debug + Send + Sync + 'static>(
        tx: &Self::Sender<T>,
        value: T,
    ) -> anyhow::Result<()> {
        Ok(tx.unbounded_send(value)?)
    }
    async fn recv<T: Send + 'static>(rx: &mut Self::Receiver<T>) -> Option<T> {
        rx.next().await
    }
}

async fn sender<UC: UnboundedChannel>(
    tx: UC::Sender<usize>,
    iteration_count: usize,
    batch_size: usize,
) {
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
                vec = UC::send_vec(&tx, vec).unwrap();
                // The intent of this benchmark is to interleave send and recv.
                yield_now().await;
            }
        }
        if !vec.is_empty() {
            _ = UC::send_vec(&tx, vec).unwrap();
        }
    }
}

async fn receiver<UC: UnboundedChannel + Send>(mut rx: UC::Receiver<usize>, batch_size: usize) {
    if batch_size == 1 {
        while let Some(_) = UC::recv(&mut rx).await {}
    } else {
        loop {
            let v = UC::recv_batch(&mut rx, batch_size).await;
            if v.is_empty() {
                break;
            }
        }
    }
}

#[divan::bench(
    types = [BatchChannel, StdChannel, CrossbeamChannel, FuturesChannel],
    consts = [1, 10, 100],
)]
fn batch_size_tx_first<UC, const N: usize>(bencher: divan::Bencher)
where
    UC: UnboundedChannel + Send + 'static,
{
    let iteration_count = 100;
    bencher
        .counter(divan::counter::ItemsCount::new(N * iteration_count))
        .with_inputs(|| {
            let pool = LocalPool::new();
            let spawner = pool.spawner();
            let (tx, rx) = UC::new();
            () = spawner.spawn(sender::<UC>(tx, iteration_count, N)).unwrap();
            () = spawner.spawn(receiver::<UC>(rx, N)).unwrap();
            pool
        })
        .bench_local_values(|mut pool| {
            pool.run_until_stalled();
        })
}

#[divan::bench(
    types = [BatchChannel, StdChannel, CrossbeamChannel, FuturesChannel],
    consts = [1, 10, 100],
)]
fn batch_size_rx_first<UC, const N: usize>(bencher: divan::Bencher)
where
    UC: UnboundedChannel + Send + 'static,
{
    let iteration_count = 100;
    bencher
        .counter(divan::counter::ItemsCount::new(N * iteration_count))
        .with_inputs(|| {
            let pool = LocalPool::new();
            let spawner = pool.spawner();
            let (tx, rx) = UC::new();
            () = spawner.spawn(receiver::<UC>(rx, N)).unwrap();
            () = spawner.spawn(sender::<UC>(tx, iteration_count, N)).unwrap();
            pool
        })
        .bench_local_values(|mut pool| {
            pool.run_until_stalled();
        })
}

fn main() {
    divan::main()
}
