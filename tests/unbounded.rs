mod fixture;
use fixture::*;

#[test]
fn send_and_recv() {
    block_on(async move {
        let (tx, rx) = batch_channel::unbounded();
        tx.into_sync().send(10).unwrap();
        assert_eq!(Some(10), rx.recv().await);
    })
}

#[test]
fn recv_returns_none_if_sender_dropped() {
    block_on(async move {
        let (tx, rx) = batch_channel::unbounded();
        drop(tx);
        assert_eq!(None as Option<()>, rx.recv().await);
    })
}

#[test]
fn recv_returns_value_if_sender_sent_before_dropping() {
    block_on(async move {
        let (tx, rx) = batch_channel::unbounded();
        tx.into_sync().send(10).unwrap();
        assert_eq!(Some(10), rx.recv().await);
    })
}

#[test]
fn recv_wakes_when_sender_sends() {
    let (tx, rx) = batch_channel::unbounded();

    let mut pool = LocalPool::new();
    pool.spawn(async move {
        assert_eq!(Some(()), rx.recv().await);
    });

    tx.into_sync().send(()).unwrap();

    pool.run();
}

#[test]
fn recv_wakes_when_sender_drops() {
    let (tx, rx) = batch_channel::unbounded();

    let mut pool = LocalPool::new();
    pool.spawn(async move {
        assert_eq!(None as Option<()>, rx.recv().await);
    });

    pool.spawn(async move {
        drop(tx);
    });

    pool.run();
}

#[test]
fn send_fails_when_receiver_drops() {
    block_on(async move {
        let (tx, rx) = batch_channel::unbounded_sync();
        drop(rx);
        assert_eq!(Err(batch_channel::SendError(())), tx.send(()));
    })
}

#[test]
fn two_receivers_and_two_senders() {
    block_on(async move {
        let (tx1, rx1) = batch_channel::unbounded();
        let tx2 = tx1.clone();
        let rx2 = rx1.clone();
        tx1.send(1).await.unwrap();
        tx2.send(2).await.unwrap();
        assert_eq!(Some(1), rx1.recv().await);
        assert_eq!(Some(2), rx2.recv().await);
    })
}

#[test]
fn two_reads_from_one_push() {
    let mut pool = LocalPool::new();

    let (tx, rx1) = batch_channel::unbounded();
    let tx = tx.into_sync();
    let rx2 = rx1.clone();

    pool.spawn(async move {
        assert_eq!(Some(10), rx1.recv().await);
    });
    pool.spawn(async move {
        assert_eq!(Some(20), rx2.recv().await);
    });
    tx.send_iter([10, 20]).unwrap();

    pool.run()
}

#[test]
fn send_batch_wakes_both() {
    let mut pool = LocalPool::new();

    let (tx, rx1) = batch_channel::unbounded();
    let tx = tx.into_sync();
    let rx2 = rx1.clone();

    pool.spawn(async move {
        assert_eq!(Some(10), rx1.recv().await);
    });
    pool.spawn(async move {
        assert_eq!(Some(20), rx2.recv().await);
    });
    pool.spawn(async move {
        tx.send_iter([10, 20]).unwrap();
    });

    pool.run()
}

#[test]
fn send_iter_array() {
    let (tx, rx) = batch_channel::unbounded();
    let tx = tx.into_sync();
    tx.send_iter(["foo", "bar", "baz"]).unwrap();
    drop(tx);

    block_on(async move {
        assert_eq!(vec!["foo", "bar", "baz"], rx.recv_batch(4).await);
    });

    let (tx, rx) = batch_channel::unbounded();
    let tx = tx.into_sync();
    drop(rx);
    assert_eq!(
        Err(batch_channel::SendError(())),
        tx.send_iter(["foo", "bar", "baz"])
    );
}

#[test]
fn send_iter_vec() {
    let (tx, rx) = batch_channel::unbounded();
    tx.into_sync().send_iter(vec!["foo", "bar", "baz"]).unwrap();

    block_on(async move {
        assert_eq!(vec!["foo", "bar", "baz"], rx.recv_batch(4).await);
    });
}

#[test]
fn recv_batch_returning_all() {
    let (tx, rx) = batch_channel::unbounded();

    tx.into_sync().send_iter([10, 20, 30]).unwrap();
    block_on(async move {
        assert_eq!(vec![10, 20, 30], rx.recv_batch(100).await);
    })
}

#[test]
fn recv_batch_returning_some() {
    let (tx, rx) = batch_channel::unbounded();

    tx.into_sync().send_iter([10, 20, 30]).unwrap();
    block_on(async move {
        assert_eq!(vec![10, 20], rx.recv_batch(2).await);
        assert_eq!(vec![30], rx.recv_batch(2).await);
    })
}

#[test]
fn recv_vec_returning_some() {
    let (tx, rx) = batch_channel::unbounded();

    tx.into_sync().send_iter([10, 20, 30]).unwrap();
    block_on(async move {
        let mut vec = Vec::new();
        () = rx.recv_vec(2, &mut vec).await;
        assert_eq!(vec![10, 20], vec);
        () = rx.recv_vec(2, &mut vec).await;
        assert_eq!(vec![30], vec);
    });
}

#[test]
fn recv_batch_returns_empty_when_no_tx() {
    let (tx, rx) = batch_channel::unbounded();
    drop(tx);

    block_on(async move {
        assert_eq!(Vec::<()>::new(), rx.recv_batch(2).await);
    })
}

#[test]
fn batch_locally_accumulates() {
    let mut pool = LocalPool::new();

    let (tx, rx) = batch_channel::unbounded();
    let read_values: StateVar<Vec<_>> = StateVar::default();

    pool.spawn({
        let read_values = read_values.clone();
        async move {
            while let Some(v) = rx.recv().await {
                read_values.borrow_mut().push(v);
            }
        }
    });

    tx.into_sync()
        .autobatch(2, |tx| {
            assert_eq!(Ok(()), tx.send(1));
            pool.run_until_stalled();
            assert_eq!(0, read_values.borrow().len());

            assert_eq!(Ok(()), tx.send(2));
            pool.run_until_stalled();
            assert_eq!(vec![1, 2], read_values.get());

            assert_eq!(Ok(()), tx.send(3));
            pool.run_until_stalled();
            assert_eq!(vec![1, 2], read_values.get());
            Ok(())
        })
        .unwrap();

    pool.run();
    assert_eq!(vec![1, 2, 3], read_values.get());
}

#[test]
fn autobatch_locally_accumulates() {
    let mut pool = LocalPool::new();

    let (tx, rx) = batch_channel::unbounded();
    let read_values: StateVar<Vec<_>> = StateVar::default();

    pool.spawn({
        let read_values = read_values.clone();
        async move {
            while let Some(v) = rx.recv().await {
                read_values.borrow_mut().push(v);
            }
        }
    });

    tx.into_sync()
        .autobatch(2, |tx| {
            assert_eq!(Ok(()), tx.send(1));
            pool.run_until_stalled();
            assert_eq!(0, read_values.borrow().len());

            assert_eq!(Ok(()), tx.send(2));
            pool.run_until_stalled();
            assert_eq!(vec![1, 2], read_values.get());

            assert_eq!(Ok(()), tx.send(3));
            pool.run_until_stalled();
            assert_eq!(vec![1, 2], read_values.get());

            Ok(())
        })
        .expect("receiver never dropped");

    // autobatch drained
    pool.run();
    assert_eq!(vec![1, 2, 3], read_values.get());
}

#[test]
fn batch_can_be_drained() {
    let mut pool = LocalPool::new();

    let (tx, rx) = batch_channel::unbounded();
    let read_values: StateVar<Vec<_>> = StateVar::default();

    pool.spawn({
        let read_values = read_values.clone();
        async move {
            while let Some(v) = rx.recv().await {
                read_values.borrow_mut().push(v);
            }
        }
    });

    tx.into_sync()
        .autobatch(3, |tx| {
            assert_eq!(Ok(()), tx.send(1));
            pool.run_until_stalled();
            assert_eq!(0, read_values.borrow().len());

            assert_eq!(Ok(()), tx.send(2));
            pool.run_until_stalled();
            assert_eq!(0, read_values.borrow().len());

            assert_eq!(Ok(()), tx.drain());
            pool.run_until_stalled();
            assert_eq!(vec![1, 2], read_values.get());

            assert_eq!(Ok(()), tx.send(3));
            pool.run_until_stalled();
            assert_eq!(2, read_values.borrow().len());

            Ok(())
        })
        .unwrap();

    pool.run();
    assert_eq!(vec![1, 2, 3], read_values.get());
}

#[test]
fn sender_and_receiver_of_noncloneable_can_clone() {
    struct NoClone;
    let (tx, rx) = batch_channel::unbounded::<NoClone>();
    _ = tx.clone();
    _ = rx.clone();
}

#[test]
fn blocking_recv_stress_condvar() {
    use std::sync::Arc;
    use std::sync::Barrier;
    use std::thread;

    const N: usize = 100;
    const I: usize = 20;
    let mut handles = Vec::with_capacity(N * 2);
    let barrier = Arc::new(Barrier::new(N * 2));
    for _ in 0..N {
        let tx_barrier = Arc::clone(&barrier);
        let rx_barrier = Arc::clone(&barrier);
        let (tx, rx) = batch_channel::unbounded_sync();
        handles.push(thread::spawn(move || {
            tx_barrier.wait();
            for i in 0..I {
                tx.send(i).unwrap();
            }
        }));
        handles.push(thread::spawn(move || {
            rx_barrier.wait();
            for i in 0..I {
                assert_eq!(Some(i), rx.recv());
            }
            assert_eq!(None, rx.recv());
        }));
    }

    for handle in handles {
        handle.join().unwrap();
    }
}

#[test]
fn recv_batch_blocking() {
    let (tx, rx) = batch_channel::unbounded_sync();
    tx.send(10).unwrap();
    tx.send(20).unwrap();
    assert_eq!(vec![10, 20], rx.recv_batch(4));
}
