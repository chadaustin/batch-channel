pub use futures::executor::block_on;
pub use futures::executor::LocalPool;
pub use futures::task::LocalSpawnExt;
use futures::Future;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::Mutex;

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

/// A mutable slot so asynchronous single-threaded tests can assert
/// progress.
#[derive(Clone, Debug)]
pub struct StateVar<T>(Rc<RefCell<T>>);

#[allow(dead_code)]
impl<T> StateVar<T> {
    pub fn new(init: T) -> Self {
        Self(Rc::new(RefCell::new(init)))
    }

    pub fn set(&self, value: T) {
        *self.0.borrow_mut() = value;
    }

    pub fn borrow(&self) -> std::cell::Ref<'_, T> {
        self.0.borrow()
    }

    pub fn borrow_mut(&self) -> std::cell::RefMut<'_, T> {
        self.0.borrow_mut()
    }
}

#[allow(dead_code)]
impl<T: Clone> StateVar<T> {
    pub fn get(&self) -> T {
        self.0.borrow().clone()
    }
}

#[allow(dead_code)]
impl<T: Default> StateVar<T> {
    pub fn default() -> Self {
        Self::new(Default::default())
    }
}

/// A mutable, atomic slot so asynchronous tests can assert progress.
#[derive(Clone, Debug)]
// Could use crossbeam::AtomicCell
pub struct AtomicVar<T>(Arc<Mutex<T>>);

#[allow(dead_code)]
impl<T> AtomicVar<T> {
    pub fn new(init: T) -> Self {
        Self(Arc::new(Mutex::new(init)))
    }

    pub fn set(&self, value: T) {
        *self.0.lock().unwrap() = value;
    }
}

#[allow(dead_code)]
impl<T: Clone> AtomicVar<T> {
    pub fn get(&self) -> T {
        self.0.lock().unwrap().clone()
    }
}
