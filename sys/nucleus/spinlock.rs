pub use core::sync::atomic::AtomicUsize;

pub struct Spinlock<T> {
    /// Marks the lock state.
    /// bit 0: locked
    /// bit 1: reserved
    lock: AtomicUsize,
    data: T,
}
unsafe impl<T> Sync for Spinlock<T> {}
impl<T> Spinlock {
    pub const fn new(data: T) -> Self {
        Self {
            lock: AtomicUsize::new(0),
            data,
        }
    }
}
