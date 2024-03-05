use divan::Bencher;

#[divan::bench]
fn batch_channel(bencher: Bencher) {
    let item_count = 1000000usize;
    bencher
        .counter(divan::counter::ItemsCount::new(item_count))
        .with_inputs(|| batch_channel::bounded_sync(item_count))
        .bench_local_values(|(tx, rx)| {
            for i in 0..item_count {
                tx.send(i).unwrap();
            }
            drop(tx);
            while let Some(_) = rx.recv() {}
        });
}

#[divan::bench]
fn kanal(bencher: Bencher) {
    let item_count = 1000000usize;
    bencher
        .counter(divan::counter::ItemsCount::new(item_count))
        .with_inputs(|| kanal::bounded(item_count))
        .bench_local_values(|(tx, rx)| {
            for i in 0..item_count {
                tx.send(i).unwrap();
            }
            drop(tx);
            while let Ok(_) = rx.recv() {}
        });
}

#[divan::bench]
fn crossbeam(bencher: Bencher) {
    let item_count = 1000000usize;
    bencher
        .counter(divan::counter::ItemsCount::new(item_count))
        .with_inputs(|| crossbeam::channel::bounded(item_count))
        .bench_local_values(|(tx, rx)| {
            for i in 0..item_count {
                tx.send(i).unwrap();
            }
            drop(tx);
            while let Ok(_) = rx.recv() {}
        });
}

fn main() {
    divan::main()
}
