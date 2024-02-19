#[test]
fn sync_to_async_and_back() {
    let (tx, rx) = batch_channel::bounded::<()>(1);

    let tx = tx.into_sync();
    let rx = rx.into_sync();

    let tx = tx.into_async();
    let rx = rx.into_async();

    _ = (tx, rx);
}
