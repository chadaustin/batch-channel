#![cfg(feature = "futures-core")]
#![allow(non_snake_case)]

use futures::StreamExt;
use std::pin::pin;
mod fixture;
use fixture::*;

#[test]
fn receiver_implements_Stream() {
    let (tx, rx) = batch_channel::bounded(1);

    let mut pool = LocalPool::new();
    pool.spawn({
        async move {
            let mut rx = pin!(rx.stream());
            assert_eq!(Some(1), rx.next().await);
            assert_eq!(Some(2), rx.next().await);
            assert_eq!(Some(3), rx.next().await);
            assert_eq!(None, rx.next().await);
        }
    });
    pool.spawn({
        async move {
            tx.send_iter([1, 2, 3]).await.unwrap();
        }
    });

    pool.run();
}
