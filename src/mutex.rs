#[cfg(not(feature = "parking_lot"))]
mod wrap {
    use std::sync;

    #[derive(Debug, Default)]
    pub struct Mutex<T>(sync::Mutex<T>);

    impl<T> Mutex<T> {
        pub fn new(t: T) -> Self {
            Mutex(sync::Mutex::new(t))
        }

        pub fn lock(&self) -> MutexGuard<'_, T> {
            self.0.lock().unwrap()
        }
    }

    pub type MutexGuard<'a, T> = sync::MutexGuard<'a, T>;

    #[derive(Debug, Default)]
    pub struct Condvar(sync::Condvar);

    impl Condvar {
        pub fn notify_all(&self) {
            self.0.notify_all()
        }

        pub fn wait_while<'a, T, F>(
            &self,
            guard: MutexGuard<'a, T>,
            condition: F,
        ) -> MutexGuard<'a, T>
        where
            F: FnMut(&mut T) -> bool,
        {
            self.0.wait_while(guard, condition).unwrap()
        }
    }
}

#[cfg(feature = "parking_lot")]
mod wrap {
    pub use parking_lot::Mutex;
    pub use parking_lot::MutexGuard;

    #[derive(Debug, Default)]
    pub struct Condvar(parking_lot::Condvar);

    impl Condvar {
        pub fn notify_all(&self) {
            self.0.notify_all();
        }

        pub fn wait_while<'a, T, F>(
            &self,
            mut guard: MutexGuard<'a, T>,
            condition: F,
        ) -> MutexGuard<'a, T>
        where
            F: FnMut(&mut T) -> bool,
        {
            self.0.wait_while(&mut guard, condition);
            guard
        }
    }
}

pub use wrap::Condvar;
pub use wrap::Mutex;
pub use wrap::MutexGuard;
