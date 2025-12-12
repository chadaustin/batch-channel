#[test]
fn receiver_implements_iterator() {
    let (tx, rx) = batch_channel::bounded(3);
    let tx = tx.into_sync();
    let rx = rx.into_sync();

    tx.send_iter(['a', 'b', 'c']).unwrap();
    drop(tx);

    assert_eq!(vec!['a', 'b', 'c'], rx.collect::<Vec<_>>());
}
