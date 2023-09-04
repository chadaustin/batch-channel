pub use futures::executor::block_on;
pub use futures::executor::LocalPool;
pub use futures::task::LocalSpawnExt;
use futures::Future;

pub trait TestPool {
    fn spawn<F: Future<Output = ()> + 'static>(&self, future: F);

    fn block_on<T, F: Future<Output = T> + 'static>(&mut self, future: F) -> T;
}

impl TestPool for LocalPool {
    fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + 'static,
    {
        self.spawner().spawn_local(future).unwrap();
    }

    fn block_on<T, F>(&mut self, future: F) -> T
    where
        F: Future<Output = T> + 'static,
    {
        self.run_until(future)
    }
}
