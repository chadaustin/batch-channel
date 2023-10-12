#[divan::bench]
fn alloc_unbounded() -> (batch_channel::Sender<u8>, batch_channel::Receiver<u8>) {
    batch_channel::unbounded()
}

#[divan::bench]
fn alloc_dealloc_unbounded() {
    let _ = batch_channel::unbounded::<u8>();
}

fn main() {
    divan::main()
}
