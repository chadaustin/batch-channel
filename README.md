async MPMC channel that reduces overhead by reading and writing many
values at once.

Sometimes large volumes of small values are farmed out to workers
through a channel. Consider directory traversal: each readdir()
call produces a batch of directory entries. Batching can help the
consumption side too. Consider querying SQLite with received
values -- larger batches reduce the amortized cost of acquiring
SQLite's lock and bulk queries can be issued.

```rust
# futures::executor::block_on(async {
let (tx, rx) = batch_channel::unbounded();

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

One can imagine natural, currently unimplemented, extensions to this
crate:
* Synchronous channels
* Bounded channels
* Channels with priority
* impls for `futures::sink::Sink` and `futures::stream::Stream`

Ask if they would be helpful.
