use core::cell::UnsafeCell;
use crossbeam_deque::{Stealer, Worker};

pub(super) struct LocalWorker<T>
where
    T: Send,
{
    worker: UnsafeCell<Worker<T>>,
}

impl<T> LocalWorker<T>
where
    T: Send,
{
    pub fn new_fifo() -> Self {
        Self {
            worker: UnsafeCell::new(Worker::new_fifo()),
        }
    }

    pub fn stealer(&self) -> Stealer<T> {
        // SAFETY: `stealer` creation is read-only and can happen from any thread.
        unsafe { (&*self.worker.get()).stealer() }
    }

    pub fn as_owner_worker(&self) -> &Worker<T> {
        // SAFETY: owner-only access is enforced by thread-to-slot assignment.
        unsafe { &*self.worker.get() }
    }
}

// SAFETY: the underlying deque supports one owner thread and many stealers.
// ConcurrentRecursiveIterCrossbeam assigns one owner per local slot and exposes only `Stealer` to other threads.
unsafe impl<T> Send for LocalWorker<T> where T: Send {}
// SAFETY: see `Send` rationale above.
unsafe impl<T> Sync for LocalWorker<T> where T: Send {}
