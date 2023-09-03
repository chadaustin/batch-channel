#![allow(non_snake_case)]

use batch_channel::SendError;
use std::cell::RefCell;
use std::rc::Rc;

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
