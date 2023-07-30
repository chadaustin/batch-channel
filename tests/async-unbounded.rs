use futures::executor::LocalPool;
use futures::task::SpawnExt;
use std::sync::Arc;
use std::sync::Mutex;

#[test]
fn send_and_recv() {
    let mut pool = LocalPool::new();
    pool.run_until(async move {
        let (tx, rx) = batch_channel::unbounded();
        tx.send(10).unwrap();
        assert_eq!(Some(10), rx.recv().await);
    })
}

#[test]
fn recv_returns_none_if_sender_dropped() {
    let mut pool = LocalPool::new();
    pool.run_until(async move {
        let (tx, rx) = batch_channel::unbounded();
        drop(tx);
        assert_eq!(None as Option<()>, rx.recv().await);
    })
}

#[test]
fn recv_returns_value_if_sender_sent_before_dropping() {
    let mut pool = LocalPool::new();
    pool.run_until(async move {
        let (tx, rx) = batch_channel::unbounded();
        tx.send(10).unwrap();
        drop(tx);
        assert_eq!(Some(10), rx.recv().await);
    })
}

#[test]
fn recv_wakes_when_sender_sends() {
    let (tx, rx) = batch_channel::unbounded();

    let mut pool = LocalPool::new();
    let spawner = pool.spawner();
    spawner
        .spawn(async move {
            assert_eq!(Some(()), rx.recv().await);
        })
        .unwrap();

    spawner
        .spawn(async move {
            tx.send(()).unwrap();
        })
        .unwrap();

    pool.run();
}

#[test]
fn recv_wakes_when_sender_drops() {
    let (tx, rx) = batch_channel::unbounded();

    let mut pool = LocalPool::new();
    let spawner = pool.spawner();
    spawner
        .spawn(async move {
            assert_eq!(None as Option<()>, rx.recv().await);
        })
        .unwrap();

    spawner
        .spawn(async move {
            drop(tx);
        })
        .unwrap();

    pool.run();
}

#[test]
fn send_fails_when_receiver_drops() {
    let mut pool = LocalPool::new();
    pool.run_until(async move {
        let (tx, rx) = batch_channel::unbounded();
        drop(rx);
        assert_eq!(Err(batch_channel::SendError(())), tx.send(()));
    })
}

#[test]
fn two_receivers_and_two_senders() {
    let mut pool = LocalPool::new();
    pool.run_until(async move {
        let (tx1, rx1) = batch_channel::unbounded();
        let tx2 = tx1.clone();
        let rx2 = rx1.clone();
        tx1.send(1).unwrap();
        tx2.send(2).unwrap();
        assert_eq!(Some(1), rx1.recv().await);
        assert_eq!(Some(2), rx2.recv().await);
    })
}

#[test]
fn two_reads_from_one_push() {
    let mut pool = LocalPool::new();
    let spawner = pool.spawner();

    let (tx, rx1) = batch_channel::unbounded();
    let rx2 = rx1.clone();

    spawner
        .spawn(async move {
            assert_eq!(Some(10), rx1.recv().await);
        })
        .unwrap();
    spawner
        .spawn(async move {
            assert_eq!(Some(20), rx2.recv().await);
        })
        .unwrap();
    tx.send_many([10, 20]).unwrap();

    pool.run()
}

#[test]
fn push_many_wakes_both() {
    let mut pool = LocalPool::new();
    let spawner = pool.spawner();

    let (tx, rx1) = batch_channel::unbounded();
    let rx2 = rx1.clone();

    spawner
        .spawn(async move {
            assert_eq!(Some(10), rx1.recv().await);
        })
        .unwrap();
    spawner
        .spawn(async move {
            assert_eq!(Some(20), rx2.recv().await);
        })
        .unwrap();
    spawner
        .spawn(async move {
            tx.send_many([10, 20]).unwrap();
        })
        .unwrap();

    pool.run()
}

#[test]
fn recv_many_returning_all() {
    let mut pool = LocalPool::new();
    let spawner = pool.spawner();

    let (tx, rx) = batch_channel::unbounded();

    tx.send_many([10, 20, 30]).unwrap();
    spawner
        .spawn(async move {
            assert_eq!(vec![10, 20, 30], rx.recv_many(100).await);
        })
        .unwrap();

    pool.run();
}

#[test]
fn recv_many_returning_some() {
    let mut pool = LocalPool::new();
    let spawner = pool.spawner();

    let (tx, rx) = batch_channel::unbounded();

    tx.send_many([10, 20, 30]).unwrap();
    spawner
        .spawn(async move {
            assert_eq!(vec![10, 20], rx.recv_many(2).await);
            assert_eq!(vec![30], rx.recv_many(2).await);
        })
        .unwrap();

    pool.run();
}

#[test]
fn recv_many_returns_empty_when_no_tx() {
    let mut pool = LocalPool::new();
    let spawner = pool.spawner();

    let (tx, rx) = batch_channel::unbounded();
    drop(tx);

    spawner
        .spawn(async move {
            assert_eq!(Vec::<()>::new(), rx.recv_many(2).await);
        })
        .unwrap();

    pool.run();
}

#[test]
fn batch_locally_accumulates() {
    let mut pool = LocalPool::new();
    let spawner = pool.spawner();

    let (tx, rx) = batch_channel::unbounded();
    let read_values = Arc::new(Mutex::new(Vec::new()));
    let read_values_outer = read_values.clone();

    spawner
        .spawn(async move {
            while let Some(v) = rx.recv().await {
                read_values.lock().unwrap().push(v);
            }
        })
        .unwrap();

    let read_values = read_values_outer;

    let mut tx = tx.batch(2);

    assert_eq!(Ok(()), tx.send(1));
    pool.run_until_stalled();
    assert_eq!(0, read_values.lock().unwrap().len());

    assert_eq!(Ok(()), tx.send(2));
    pool.run_until_stalled();
    assert_eq!(vec![1, 2], *read_values.lock().unwrap());

    assert_eq!(Ok(()), tx.send(3));
    pool.run_until_stalled();
    assert_eq!(vec![1, 2], *read_values.lock().unwrap());
    drop(tx);
    pool.run_until_stalled();
    assert_eq!(vec![1, 2, 3], *read_values.lock().unwrap());
}
