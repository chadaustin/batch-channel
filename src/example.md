```rust
# futures::executor::block_on(async {
let (tx, rx) = batch_channel::unbounded();
let tx = tx.into_sync();

# let value = 8675309;
# let v1 = 1;
# let v2 = 2;
# let v3 = 3;

tx.send(value).unwrap();
tx.send_iter([v1, v2, v3]).unwrap();

match rx.recv().await {
  Some(value) => println!("single {value}"),
  None => return "sender closed",
}

let batch = rx.recv_batch(100).await;
// batch.is_empty() means sender closed
# ""
# });
```
