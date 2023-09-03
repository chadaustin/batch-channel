use tiny_bench::bench;
use tiny_bench::black_box;

fn main() {
    bench(|| {
        _ = black_box(batch_channel::unbounded::<u8>());
    });
}
