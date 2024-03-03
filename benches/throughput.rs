use futures::future::BoxFuture;
use futures::FutureExt;
use std::future::Future;
use std::time::Instant;

trait Channel {
    const HAS_BATCH: bool;

    type Sender<T: Send + 'static>: ChannelSender<T> + 'static;
    type Receiver<T: Send + 'static>: ChannelReceiver<T> + 'static;

    fn bounded<T: Send + 'static>(capacity: usize) -> (Self::Sender<T>, Self::Receiver<T>);
}

trait ChannelSync {
    const HAS_BATCH: bool;

    type SyncSender<T: Send + 'static>: ChannelSyncSender<T> + 'static;
    type SyncReceiver<T: Send + 'static>: ChannelSyncReceiver<T> + 'static;

    fn bounded_sync<T: Send + 'static>(
        capacity: usize,
    ) -> (Self::SyncSender<T>, Self::SyncReceiver<T>);
}

trait ChannelSender<T>: Clone + Send {
    type BatchSender: ChannelBatchSender<T>;

    fn autobatch<F>(self, batch_limit: usize, f: F) -> impl Future<Output = ()> + Send
    where
        for<'a> F: (FnOnce(&'a mut Self::BatchSender) -> BoxFuture<'a, ()>) + Send + 'static;
}

trait ChannelBatchSender<T>: Send {
    fn send(&mut self, value: T) -> impl Future<Output = ()> + Send;
}

trait ChannelReceiver<T>: Clone + Send {
    fn recv_vec<'a>(
        &'a self,
        element_limit: usize,
        vec: &'a mut Vec<T>,
    ) -> impl Future<Output = ()> + Send;
}

trait ChannelSyncSender<T>: Clone + Send {
    type BatchSenderSync: ChannelBatchSenderSync<T>;

    fn autobatch<F>(self, batch_limit: usize, f: F)
    where
        F: FnOnce(&mut Self::BatchSenderSync);
}

trait ChannelBatchSenderSync<T>: Send {
    fn send(&mut self, value: T);
}

trait ChannelSyncReceiver<T>: Clone + Send {
    fn recv_vec(&self, element_limit: usize, vec: &mut Vec<T>);
}

// batch-channel, this crate

struct BatchChannel;

impl Channel for BatchChannel {
    const HAS_BATCH: bool = true;

    type Sender<T: Send + 'static> = batch_channel::Sender<T>;
    type Receiver<T: Send + 'static> = batch_channel::Receiver<T>;

    fn bounded<T: Send + 'static>(capacity: usize) -> (Self::Sender<T>, Self::Receiver<T>) {
        batch_channel::bounded(capacity)
    }
}

impl ChannelSync for BatchChannel {
    const HAS_BATCH: bool = true;

    type SyncSender<T: Send + 'static> = batch_channel::SyncSender<T>;
    type SyncReceiver<T: Send + 'static> = batch_channel::SyncReceiver<T>;

    fn bounded_sync<T: Send + 'static>(
        capacity: usize,
    ) -> (Self::SyncSender<T>, Self::SyncReceiver<T>) {
        batch_channel::bounded_sync(capacity)
    }
}

impl<T: Send> ChannelSender<T> for batch_channel::Sender<T> {
    type BatchSender = batch_channel::BatchSender<T>;

    fn autobatch<F>(self, batch_limit: usize, f: F) -> impl Future<Output = ()> + Send
    where
        for<'a> F: (FnOnce(&'a mut Self::BatchSender) -> BoxFuture<'a, ()>) + Send + 'static,
    {
        async move {
            batch_channel::Sender::autobatch(self, batch_limit, |tx| {
                async move {
                    () = f(tx).await;
                    Ok(())
                }
                .boxed()
            })
            .await
            .expect("in this benchmark, receiver never drops")
        }
    }
}

impl<T: Send> ChannelBatchSender<T> for batch_channel::BatchSender<T> {
    fn send(&mut self, value: T) -> impl Future<Output = ()> + Send {
        async move {
            batch_channel::BatchSender::send(self, value)
                .await
                .expect("in this benchmark, receiver never drops")
        }
    }
}

impl<T: Send> ChannelReceiver<T> for batch_channel::Receiver<T> {
    fn recv_vec<'a>(
        &'a self,
        element_limit: usize,
        vec: &'a mut Vec<T>,
    ) -> impl Future<Output = ()> + Send {
        batch_channel::Receiver::recv_vec(self, element_limit, vec)
    }
}

impl<T: Send> ChannelSyncSender<T> for batch_channel::SyncSender<T> {
    type BatchSenderSync = batch_channel::SyncBatchSender<T>;

    fn autobatch<F>(self, batch_limit: usize, f: F)
    where
        F: FnOnce(&mut Self::BatchSenderSync),
    {
        batch_channel::SyncSender::autobatch(self, batch_limit, |tx| {
            f(tx);
            Ok(())
        })
        .expect("in this benchmark, receiver never drops")
    }
}

impl<T: Send> ChannelBatchSenderSync<T> for batch_channel::SyncBatchSender<T> {
    fn send(&mut self, value: T) {
        batch_channel::SyncBatchSender::send(self, value)
            .expect("in this benchmark, receiver never drops")
    }
}

impl<T: Send> ChannelSyncReceiver<T> for batch_channel::SyncReceiver<T> {
    fn recv_vec(&self, element_limit: usize, vec: &mut Vec<T>) {
        batch_channel::SyncReceiver::recv_vec(self, element_limit, vec)
    }
}

// Kanal

struct KanalChannel;

impl Channel for KanalChannel {
    const HAS_BATCH: bool = false;

    type Sender<T: Send + 'static> = kanal::AsyncSender<T>;
    type Receiver<T: Send + 'static> = kanal::AsyncReceiver<T>;

    fn bounded<T: Send + 'static>(capacity: usize) -> (Self::Sender<T>, Self::Receiver<T>) {
        kanal::bounded_async(capacity)
    }
}

impl<T: Send + 'static> ChannelSender<T> for kanal::AsyncSender<T> {
    type BatchSender = kanal::AsyncSender<T>;

    fn autobatch<F>(mut self, _batch_limit: usize, f: F) -> impl Future<Output = ()> + Send
    where
        for<'a> F: (FnOnce(&'a mut Self::BatchSender) -> BoxFuture<'a, ()>) + Send + 'static,
    {
        async move {
            f(&mut self).await;
        }
    }
}

impl<T: Send> ChannelBatchSender<T> for kanal::AsyncSender<T> {
    fn send(&mut self, value: T) -> impl Future<Output = ()> + Send {
        async move {
            kanal::AsyncSender::send(self, value)
                .await
                .expect("in this benchmark, receiver never drops")
        }
    }
}

impl<T: Send> ChannelReceiver<T> for kanal::AsyncReceiver<T> {
    fn recv_vec<'a>(
        &'a self,
        element_limit: usize,
        vec: &'a mut Vec<T>,
    ) -> impl Future<Output = ()> + Send {
        async move {
            let Ok(value) = self.recv().await else {
                return;
            };
            vec.push(value);
            // Now try to read the rest.
            for _ in 0..element_limit {
                let Ok(Some(value)) = self.try_recv() else {
                    return;
                };
                vec.push(value);
            }
        }
    }
}

impl ChannelSync for KanalChannel {
    const HAS_BATCH: bool = false;

    type SyncSender<T: Send + 'static> = kanal::Sender<T>;
    type SyncReceiver<T: Send + 'static> = kanal::Receiver<T>;

    fn bounded_sync<T: Send + 'static>(
        capacity: usize,
    ) -> (Self::SyncSender<T>, Self::SyncReceiver<T>) {
        kanal::bounded(capacity)
    }
}

impl<T: Send> ChannelSyncSender<T> for kanal::Sender<T> {
    type BatchSenderSync = kanal::Sender<T>;

    fn autobatch<F>(mut self, _batch_limit: usize, f: F)
    where
        for<'a> F: FnOnce(&'a mut Self::BatchSenderSync),
    {
        f(&mut self);
    }
}

impl<T: Send> ChannelBatchSenderSync<T> for kanal::Sender<T> {
    fn send(&mut self, value: T) {
        kanal::Sender::send(self, value).expect("in this benchmark, receiver never drops")
    }
}

impl<T: Send> ChannelSyncReceiver<T> for kanal::Receiver<T> {
    fn recv_vec(&self, element_limit: usize, vec: &mut Vec<T>) {
        let Ok(value) = self.recv() else {
            return;
        };
        vec.push(value);
        // Now try to read the rest.
        for _ in 1..element_limit {
            let Ok(Some(value)) = self.try_recv() else {
                return;
            };
            vec.push(value);
        }
    }
}

// Crossbeam

struct CrossbeamChannel;

impl ChannelSync for CrossbeamChannel {
    const HAS_BATCH: bool = false;

    type SyncSender<T: Send + 'static> = crossbeam::channel::Sender<T>;
    type SyncReceiver<T: Send + 'static> = crossbeam::channel::Receiver<T>;

    fn bounded_sync<T: Send + 'static>(
        capacity: usize,
    ) -> (Self::SyncSender<T>, Self::SyncReceiver<T>) {
        crossbeam::channel::bounded(capacity)
    }
}

impl<T: Send> ChannelSyncSender<T> for crossbeam::channel::Sender<T> {
    type BatchSenderSync = crossbeam::channel::Sender<T>;

    fn autobatch<F>(mut self, _batch_limit: usize, f: F)
    where
        for<'a> F: FnOnce(&'a mut Self::BatchSenderSync),
    {
        f(&mut self);
    }
}

impl<T: Send> ChannelBatchSenderSync<T> for crossbeam::channel::Sender<T> {
    fn send(&mut self, value: T) {
        crossbeam::channel::Sender::send(self, value)
            .expect("in this benchmark, receiver never drops")
    }
}

impl<T: Send> ChannelSyncReceiver<T> for crossbeam::channel::Receiver<T> {
    fn recv_vec(&self, element_limit: usize, vec: &mut Vec<T>) {
        let Ok(value) = self.recv() else {
            return;
        };
        vec.push(value);
        // Now try to read the rest.
        for _ in 1..element_limit {
            let Ok(value) = self.try_recv() else {
                return;
            };
            vec.push(value);
        }
    }
}

// async-channel

struct AsyncChannel;

impl Channel for AsyncChannel {
    const HAS_BATCH: bool = false;

    type Sender<T: Send + 'static> = async_channel::Sender<T>;
    type Receiver<T: Send + 'static> = async_channel::Receiver<T>;

    fn bounded<T: Send + 'static>(capacity: usize) -> (Self::Sender<T>, Self::Receiver<T>) {
        async_channel::bounded(capacity)
    }
}

impl<T: Send + 'static> ChannelSender<T> for async_channel::Sender<T> {
    type BatchSender = async_channel::Sender<T>;

    fn autobatch<F>(mut self, _batch_limit: usize, f: F) -> impl Future<Output = ()> + Send
    where
        for<'a> F: (FnOnce(&'a mut Self::BatchSender) -> BoxFuture<'a, ()>) + Send + 'static,
    {
        async move {
            f(&mut self).await;
        }
    }
}

impl<T: Send> ChannelBatchSender<T> for async_channel::Sender<T> {
    fn send(&mut self, value: T) -> impl Future<Output = ()> + Send {
        async move {
            async_channel::Sender::send(self, value)
                .await
                .expect("in this benchmark, receiver never drops")
        }
    }
}

impl<T: Send> ChannelReceiver<T> for async_channel::Receiver<T> {
    fn recv_vec<'a>(
        &'a self,
        element_limit: usize,
        vec: &'a mut Vec<T>,
    ) -> impl Future<Output = ()> + Send {
        async move {
            let Ok(value) = self.recv().await else {
                return;
            };
            vec.push(value);
            // Now try to read the rest.
            for _ in 1..element_limit {
                let Ok(value) = self.try_recv() else {
                    return;
                };
                vec.push(value);
            }
        }
    }
}

// Benchmark

#[derive(Copy, Clone)]
struct Options {
    batch_size: usize,
    tx_count: usize,
    rx_count: usize,
}

async fn benchmark_throughput_async<C, SpawnTx, SpawnRx>(
    _: C,
    options: Options,
    spawn_tx: SpawnTx,
    spawn_rx: SpawnRx,
) where
    C: Channel,
    SpawnTx: Fn(BoxFuture<'static, ()>) -> tokio::task::JoinHandle<()>,
    SpawnRx: Fn(BoxFuture<'static, ()>) -> tokio::task::JoinHandle<()>,
{
    const CAPACITY: usize = 65536;
    let send_count: usize = 2 * 1024 * 1024 * (if C::HAS_BATCH { options.batch_size } else { 1 });
    let total_items = send_count * options.tx_count;

    let mut senders = Vec::new();
    let mut receivers = Vec::new();

    let now = Instant::now();

    let (tx, rx) = C::bounded(CAPACITY);
    for task_id in 0..options.tx_count {
        let tx = tx.clone();
        senders.push(spawn_tx(
            async move {
                tx.autobatch(options.batch_size, move |tx| {
                    async move {
                        for i in 0..send_count {
                            tx.send((task_id, i)).await;
                        }
                    }
                    .boxed()
                })
                .await;
            }
            .boxed(),
        ));
    }
    drop(tx);
    for _ in 0..options.rx_count {
        let rx = rx.clone();
        receivers.push(spawn_rx(
            async move {
                let mut batch = Vec::with_capacity(options.batch_size);
                loop {
                    batch.clear();
                    rx.recv_vec(options.batch_size, &mut batch).await;
                    if batch.is_empty() {
                        break;
                    }
                }
            }
            .boxed(),
        ));
    }
    drop(rx);

    for r in receivers {
        () = r.await.expect("task panicked");
    }
    for s in senders {
        () = s.await.expect("task panicked");
    }

    let elapsed = now.elapsed();
    println!(
        "{:?}, {:?} per item",
        elapsed,
        elapsed / (total_items as u32)
    );
}

fn benchmark_throughput_sync<C>(_: C, options: Options)
where
    C: ChannelSync,
{
    const CAPACITY: usize = 65536;
    let send_count: usize = 1 * 1024 * 1024 * (if C::HAS_BATCH { options.batch_size } else { 1 });
    let total_items = send_count * options.tx_count;

    let mut senders = Vec::new();
    let mut receivers = Vec::new();

    let now = Instant::now();

    let (tx, rx) = C::bounded_sync(CAPACITY);
    for task_id in 0..options.tx_count {
        let tx = tx.clone();
        senders.push(std::thread::spawn(move || {
            tx.autobatch(options.batch_size, move |tx| {
                for i in 0..send_count {
                    tx.send((task_id, i));
                }
            })
        }));
    }
    drop(tx);
    for _ in 0..options.rx_count {
        let rx = rx.clone();
        receivers.push(std::thread::spawn(move || {
            let mut batch = Vec::with_capacity(options.batch_size);
            loop {
                batch.clear();
                rx.recv_vec(options.batch_size, &mut batch);
                if batch.is_empty() {
                    break;
                }
            }
        }));
    }
    drop(rx);

    for r in receivers {
        () = r.join().expect("should not complete");
    }

    let elapsed = now.elapsed();
    println!(
        "{:?}, {:?} per item",
        elapsed,
        elapsed / (total_items as u32)
    );
}

fn main() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(8)
        .build()
        .expect("failed to create tokio runtime");

    let batch_sizes = [1, 2, 4, 8, 16, 32, 64, 128, 256];

    for (tx_count, rx_count) in [(1, 1), (4, 1), (4, 4)] {
        println!();
        println!("throughput async (tx={} rx={})", tx_count, rx_count);
        for batch_size in batch_sizes {
            println!("  batch={}", batch_size);
            let options = Options {
                batch_size,
                tx_count,
                rx_count,
            };
            print!("    batch-channel: ");
            runtime.block_on(benchmark_throughput_async(
                BatchChannel,
                options,
                |f| runtime.spawn(f),
                |f| runtime.spawn(f),
            ));
            print!("    kanal:         ");
            runtime.block_on(benchmark_throughput_async(
                KanalChannel,
                options,
                |f| runtime.spawn(f),
                |f| runtime.spawn(f),
            ));
            print!("    async-channel: ");
            runtime.block_on(benchmark_throughput_async(
                AsyncChannel,
                options,
                |f| runtime.spawn(f),
                |f| runtime.spawn(f),
            ));
        }

        println!();
        println!("throughput sync (tx={} rx={})", tx_count, rx_count);
        for batch_size in batch_sizes {
            println!("  batch={}", batch_size);
            let options = Options {
                batch_size,
                tx_count,
                rx_count,
            };
            print!("    batch-channel: ");
            benchmark_throughput_sync(BatchChannel, options);
            print!("    kanal:         ");
            benchmark_throughput_sync(KanalChannel, options);
            print!("    crossbeam:     ");
            benchmark_throughput_sync(CrossbeamChannel, options);
        }
    }
}
