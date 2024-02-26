use futures::future::BoxFuture;
use futures::FutureExt;
use std::future::Future;
use std::time::Instant;

trait Channel {
    type Sender<T: Send + 'static>: ChannelSender<T> + 'static;
    type Receiver<T: Send + 'static>: ChannelReceiver<T> + 'static;

    fn bounded<T: Send + 'static>(capacity: usize) -> (Self::Sender<T>, Self::Receiver<T>);
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

struct BatchChannel;

impl Channel for BatchChannel {
    type Sender<T: Send + 'static> = batch_channel::Sender<T>;
    type Receiver<T: Send + 'static> = batch_channel::Receiver<T>;

    fn bounded<T: Send + 'static>(capacity: usize) -> (Self::Sender<T>, Self::Receiver<T>) {
        batch_channel::bounded(capacity)
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
    const SEND_COUNT: usize = 16 * 1024 * 1024;
    let total_items = SEND_COUNT * options.tx_count;

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
                        for i in 0..SEND_COUNT {
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
        () = r.await.expect("should not complete");
    }

    let elapsed = now.elapsed();
    println!("    ... {:?}, {:?} per item", elapsed, elapsed / (total_items as u32));
}

fn main() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(8)
        .build()
        .expect("failed to create tokio runtime");

    println!("benchmarking throughput");
    println!();
    for batch_size in [1, 2, 4, 8, 16, 32, 64, 128, 256] {
        let options = Options {
            batch_size: batch_size,
            tx_count: 4,
            rx_count: 4,
        };
        println!("batch-channel batch={}", batch_size);
        runtime.block_on(benchmark_throughput_async(
            BatchChannel,
            options,
            |f| runtime.spawn(f),
            |f| runtime.spawn(f),
        ));
    }
}
