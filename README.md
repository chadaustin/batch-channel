async MPMC channel that reduces overhead by reading and writing many
values at once.

Sometimes large volumes of small values are farmed out to workers
through a channel. Consider directory traversal: each readdir()
call produces a batch of directory entries. Batching can help the
consumption side too. Consider querying SQLite with received
values -- larger batches reduce the amortized cost of acquiring
SQLite's lock and bulk queries can be issued.

One can imagine natural, currently unimplemented, extensions to this
crate:
* Synchronous channels
* Bounded channels
* Channels with priority
* impls for `futures::sink::Sink` and `futures::stream::Stream`

Ask if they would be helpful.
