#![allow(non_snake_case)]

use batch_channel::SendError;
use futures::FutureExt;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::Mutex;

mod fixture;
use fixture::*;

#[test]
fn zero_capacity_rounds_up_to_one() {
    let (tx, rx) = batch_channel::bounded(1);

    block_on(async move {
        tx.send(10).await.unwrap();
        assert_eq!(Some(10), rx.recv().await);
    });
}

#[test]
fn send_returns_SendError_if_receivers_dropped() {
    let (tx, rx) = batch_channel::bounded(1);
    drop(rx);
    assert_eq!(Err(SendError("x")), block_on(tx.send("x")));
}

#[test]
fn recv_unblocks_send() {
    let mut pool = LocalPool::new();
    let (tx, rx) = batch_channel::bounded(1);

    let state = Rc::new(RefCell::new(""));

    pool.spawn({
        let state = state.clone();
        async move {
            *state.borrow_mut() = "sending";
            tx.send(10).await.unwrap();
            *state.borrow_mut() = "sent 1";
            tx.send(20).await.unwrap();
            *state.borrow_mut() = "sent 2";
        }
    });

    pool.run_until_stalled();
    assert_eq!("sent 1", *state.borrow());
    assert_eq!(Some(10), block_on(rx.recv()));

    pool.run_until_stalled();
    assert_eq!("sent 2", *state.borrow());
    assert_eq!(Some(20), block_on(rx.recv()));

    pool.run();
}

#[test]
fn recv_batch_unblocks_send() {
    let mut pool = LocalPool::new();
    let (tx, rx) = batch_channel::bounded(1);

    let state = Rc::new(RefCell::new(""));

    pool.spawn({
        let state = state.clone();
        async move {
            *state.borrow_mut() = "sending";
            tx.send(10).await.unwrap();
            *state.borrow_mut() = "sent 1";
            tx.send(20).await.unwrap();
            *state.borrow_mut() = "sent 2";
        }
    });

    pool.run_until_stalled();
    assert_eq!("sent 1", *state.borrow());
    assert_eq!(vec![10], block_on(rx.recv_batch(5)));

    pool.run_until_stalled();
    assert_eq!("sent 2", *state.borrow());
    assert_eq!(vec![20], block_on(rx.recv_batch(5)));

    pool.run();
}

#[test]
fn recv_batch_returning_all() {
    let (tx, rx) = batch_channel::bounded(3);

    block_on(async move {
        tx.send_iter([10, 20, 30]).await.unwrap();
        assert_eq!(vec![10, 20, 30], rx.recv_batch(100).await);
    })
}

#[test]
fn send_batch_blocks_as_needed() {
    let mut pool = LocalPool::new();
    let (tx, rx) = batch_channel::bounded(1);

    pool.spawn(async move {
        tx.send_iter([1, 2, 3, 4]).await.unwrap();
    });

    pool.block_on(async move {
        assert_eq!(Some(1), rx.recv().await);
        assert_eq!(Some(2), rx.recv().await);
        assert_eq!(Some(3), rx.recv().await);
        assert_eq!(Some(4), rx.recv().await);
        assert_eq!(None, rx.recv().await);
    });

    pool.run();
}

#[test]
fn autobatch_batches() {
    let mut pool = LocalPool::new();
    let state = Arc::new(Mutex::new(""));

    let (tx, rx) = batch_channel::bounded(1);
    let inner = state.clone();
    pool.spawn(async move {
        tx.autobatch(2, move |tx| {
            async move {
                *inner.lock().unwrap() = "0";
                tx.send(1).await?;
                *inner.lock().unwrap() = "1";
                tx.send(2).await?;
                *inner.lock().unwrap() = "2";
                tx.send(3).await?;
                *inner.lock().unwrap() = "3";
                tx.send(4).await?;
                *inner.lock().unwrap() = "4";
                Ok(())
            }
            .boxed()
        })
        .await
        .unwrap()
    });

    pool.run_until_stalled();
    assert_eq!("1", *state.lock().unwrap());
    assert_eq!(Some(1), pool.run_until(rx.recv()));
    assert_eq!("1", *state.lock().unwrap());
    assert_eq!(Some(2), pool.run_until(rx.recv()));
    assert_eq!("3", *state.lock().unwrap());
    assert_eq!(Some(3), pool.run_until(rx.recv()));
    assert_eq!("3", *state.lock().unwrap());
    assert_eq!(Some(4), pool.run_until(rx.recv()));
    assert_eq!("4", *state.lock().unwrap());
    assert_eq!(None, pool.run_until(rx.recv()));
}

#[test]
fn autobatch_or_cancel_stops_if_receiver_is_dropped() {
    let mut pool = LocalPool::new();
    let state = Arc::new(Mutex::new(""));

    let (tx, rx) = batch_channel::bounded(1);
    let inner = state.clone();
    pool.spawn(tx.autobatch_or_cancel(2, move |tx| {
        async move {
            *inner.lock().unwrap() = "0";
            tx.send(1).await?;
            *inner.lock().unwrap() = "1";
            tx.send(2).await?;
            *inner.lock().unwrap() = "2";
            tx.send(3).await?;
            *inner.lock().unwrap() = "3";
            tx.send(4).await?;
            *inner.lock().unwrap() = "4";
            Ok(())
        }
        .boxed()
    }));

    pool.run_until_stalled();
    assert_eq!("1", *state.lock().unwrap());
    assert_eq!(Some(1), pool.run_until(rx.recv()));
    assert_eq!("1", *state.lock().unwrap());
    drop(rx);
    pool.run_until_stalled();
    assert_eq!("1", *state.lock().unwrap());
}

#[test]
fn clone_bounded_sender() {
    let mut pool = LocalPool::new();
    let (tx1, rx) = batch_channel::bounded::<()>(1);
    let tx2 = tx1.clone();
    drop(tx1);
    let tx3 = tx2.clone();
    drop(tx2);
    drop(tx3);
    assert_eq!(None, pool.run_until(rx.recv()));
}

#[test]
fn sender_and_receiver_of_noncloneable_can_clone() {
    struct NoClone;
    let (tx, rx) = batch_channel::bounded::<NoClone>(1);
    _ = tx.clone();
    _ = rx.clone();
}
