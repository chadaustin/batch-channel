use futures::{future::BoxFuture, FutureExt};
use std::future::Future;

trait ChannelBatchSender<T>: Send {
    fn send(&mut self, value: T) -> impl Future<Output = ()> + Send;
}

trait ChannelSender<T>: Clone + Send {
    type BatchSender: ChannelBatchSender<T>;

    fn autobatch<F>(self, batch_limit: usize, f: F) -> impl Future<Output = ()> + Send
    where
        for<'a> F: (FnOnce(&'a mut Self::BatchSender) -> BoxFuture<'a, ()>);
}

trait ChannelReceiver<T>: Clone + Send {
    fn recv_vec<'a>(
        &'a self,
        element_limit: usize,
        vec: &'a mut Vec<T>,
    ) -> impl Future<Output = ()> + Send;
}

trait Channel {
    type Sender<T>: ChannelSender<T>;
    type Receiver<T>: ChannelReceiver<T>;

    fn bounded<T>(capacity: usize) -> (Self::Sender<T>, Self::Receiver<T>);
}

trait JoinHandle {
    fn join(&mut self);
}

struct Options {
    batch_size: usize,
    tx_count: usize,
    rx_count: usize,
}

fn benchmark_throughput_async<C, SpawnTx, SpawnRx>(
    options: &Options,
    spawn_tx: SpawnTx,
    spawn_rx: SpawnRx,
) where
    C: Channel,
    SpawnTx: Fn(BoxFuture<()>) -> Box<dyn JoinHandle>,
    SpawnRx: Fn(BoxFuture<()>) -> Box<dyn JoinHandle>,
{
    const CAPACITY: usize = 65536;
    const BATCH_SIZE: usize = 128;
    const SEND_COUNT: usize = 2 * 1024 * 1024;

    let mut senders = Vec::new();
    let mut receivers = Vec::new();

    let (tx, rx) = C::bounded(CAPACITY);
    for task_id in 0..options.tx_count {
        let tx = tx.clone();
        senders.push(spawn_tx(
            async move {
                tx.autobatch(BATCH_SIZE, |tx| {
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
                let mut batch = Vec::with_capacity(BATCH_SIZE);
                loop {
                    rx.recv_vec(BATCH_SIZE, &mut batch).await;
                    if batch.is_empty() {
                        break;
                    }
                }
            }
            .boxed(),
        ));
    }
    drop(rx);

    for mut r in receivers {
        r.join();
    }
}

fn main() {
    println!("benchmarking throughput");
    //benchmark_throughput();
}
