use divan::Bencher;

#[divan::bench]
fn alloc_unbounded() -> (batch_channel::Sender<u8>, batch_channel::Receiver<u8>) {
    batch_channel::unbounded()
}

#[divan::bench]
fn alloc_dealloc_unbounded() {
    let _ = batch_channel::unbounded::<u8>();
}

#[divan::bench]
fn clone_rx(bencher: Bencher) {
    let (_tx, mut rx) = batch_channel::unbounded::<u8>();
    bencher.bench_local(|| {
        rx = rx.clone();
    });
}

#[divan::bench]
fn clone_tx(bencher: Bencher) {
    let (mut tx, _rx) = batch_channel::unbounded::<u8>();
    bencher.bench_local(|| {
        tx = tx.clone();
    });
}

fn main() {
    divan::main()
}
