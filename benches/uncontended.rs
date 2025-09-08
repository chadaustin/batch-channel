use divan::Bencher;

#[divan::bench]
fn batch_channel(bencher: Bencher) {
    let item_count = 1000000usize;
    bencher
        .counter(divan::counter::ItemsCount::new(item_count))
        .with_inputs(|| {
            batch_channel::Builder::new()
                .bounded(item_count)
                .preallocate()
                .build_sync()
        })
        .bench_local_values(|(tx, rx)| {
            for i in 0..item_count {
                tx.send(i).unwrap();
            }
            drop(tx);
            for i in 0..item_count {
                assert_eq!(Some(i), rx.recv());
            }
            assert_eq!(None, rx.recv());
        });
}

#[divan::bench]
fn batch_channel_send_only(bencher: Bencher) {
    let item_count = 1000000usize;
    bencher
        .counter(divan::counter::ItemsCount::new(item_count))
        .with_inputs(|| {
            batch_channel::Builder::new()
                .bounded(item_count)
                .preallocate()
                .build_sync()
        })
        .bench_local_values(|(tx, rx)| {
            for i in 0..item_count {
                tx.send(i).unwrap();
            }
            drop(tx);
            drop(rx);
        });
}

#[divan::bench]
fn batch_channel_recv_only(bencher: Bencher) {
    let item_count = 1000000usize;
    bencher
        .counter(divan::counter::ItemsCount::new(item_count))
        .with_inputs(|| {
            let (tx, rx) = batch_channel::Builder::new()
                .bounded(item_count)
                .preallocate()
                .build_sync();
            for i in 0..item_count {
                tx.send(i).unwrap();
            }
            drop(tx);
            rx
        })
        .bench_local_values(|rx| {
            for i in 0..item_count {
                assert_eq!(Some(i), rx.recv());
            }
            assert_eq!(None, rx.recv());
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
            for i in 0..item_count {
                assert_eq!(Ok(i), rx.recv());
            }
            assert!(rx.recv().is_err());
        });
}

#[divan::bench]
fn kanal_send_only(bencher: Bencher) {
    let item_count = 1000000usize;
    bencher
        .counter(divan::counter::ItemsCount::new(item_count))
        .with_inputs(|| kanal::bounded(item_count))
        .bench_local_values(|(tx, rx)| {
            for i in 0..item_count {
                tx.send(i).unwrap();
            }
            drop(tx);
            drop(rx);
        });
}

#[divan::bench]
fn kanal_recv_only(bencher: Bencher) {
    let item_count = 1000000usize;
    bencher
        .counter(divan::counter::ItemsCount::new(item_count))
        .with_inputs(|| {
            let (tx, rx) = kanal::bounded(item_count);
            for i in 0..item_count {
                tx.send(i).unwrap();
            }
            drop(tx);
            rx
        })
        .bench_local_values(|rx| {
            for i in 0..item_count {
                assert_eq!(Ok(i), rx.recv());
            }
            assert!(rx.recv().is_err());
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
            for i in 0..item_count {
                assert_eq!(Ok(i), rx.recv());
            }
            assert!(rx.recv().is_err());
        });
}

#[divan::bench]
fn crossbeam_send_only(bencher: Bencher) {
    let item_count = 1000000usize;
    bencher
        .counter(divan::counter::ItemsCount::new(item_count))
        .with_inputs(|| crossbeam::channel::bounded(item_count))
        .bench_local_values(|(tx, rx)| {
            for i in 0..item_count {
                tx.send(i).unwrap();
            }
            drop(tx);
            drop(rx);
        });
}

#[divan::bench]
fn crossbeam_recv_only(bencher: Bencher) {
    let item_count = 1000000usize;
    bencher
        .counter(divan::counter::ItemsCount::new(item_count))
        .with_inputs(|| {
            let (tx, rx) = crossbeam::channel::bounded(item_count);
            for i in 0..item_count {
                tx.send(i).unwrap();
            }
            drop(tx);
            rx
        })
        .bench_local_values(|rx| {
            for i in 0..item_count {
                assert_eq!(Ok(i), rx.recv());
            }
            assert!(rx.recv().is_err());
        });
}

fn main() {
    divan::main()
}
