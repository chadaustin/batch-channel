pub use futures::executor::block_on;
pub use futures::executor::LocalPool;
pub use futures::task::LocalSpawnExt;
use futures::Future;

pub trait TestPool {
    fn spawn<F: Future<Output = ()> + 'static>(&self, future: F);
}

impl TestPool for LocalPool {
    fn spawn<F: Future<Output = ()> + 'static>(&self, future: F) {
        self.spawner().spawn_local(future).unwrap();
    }
}
