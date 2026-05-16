use crossbeam_queue::SegQueue;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Concurrent recursive iterator using crossbeam's SegQueue.
pub struct ConcurrentIterCrossSeg<T, E>
where
    T: Send,
    E: Fn(&T) -> Vec<T> + Send + Sync,
{
    queue: Arc<SegQueue<T>>,
    extend: Arc<E>,
    pending: Arc<AtomicUsize>,
}

impl<T, E> ConcurrentIterCrossSeg<T, E>
where
    T: Send,
    E: Fn(&T) -> Vec<T> + Send + Sync,
{
    /// Create a new concurrent recursive iterator using SegQueue.
    pub fn new(initial_elements: impl IntoIterator<Item = T>, extend: E) -> Self {
        let queue = Arc::new(SegQueue::new());
        let mut pending = 0usize;
        for element in initial_elements {
            queue.push(element);
            pending += 1;
        }

        Self {
            queue,
            extend: Arc::new(extend),
            pending: Arc::new(AtomicUsize::new(pending)),
        }
    }

    /// Create a new concurrent recursive iterator with exact length.
    pub fn new_exact(
        initial_elements: impl IntoIterator<Item = T>,
        extend: E,
        _exact_len: usize,
    ) -> Self {
        Self::new(initial_elements, extend)
    }

    /// Process all elements using the specified number of threads.
    /// Returns the sum of results from each thread using the accumulator closure.
    pub fn run_with_threads<F>(&self, num_threads: usize, accumulator: F) -> u64
    where
        F: Fn(&T) -> u64 + Send + Sync,
    {
        let accumulator = Arc::new(accumulator);
        let result = Arc::new(AtomicU64::new(0));

        std::thread::scope(|scope| {
            for _ in 0..num_threads.max(1) {
                let queue_ref = Arc::clone(&self.queue);
                let pending_ref = Arc::clone(&self.pending);
                let extend_ref = Arc::clone(&self.extend);
                let result_ref = Arc::clone(&result);
                let accumulator_ref = Arc::clone(&accumulator);

                scope.spawn(move || {
                    let mut local_sum = 0u64;

                    loop {
                        if let Some(item) = queue_ref.pop() {
                            let children = extend_ref(&item);
                            let num_children = children.len();
                            if num_children > 0 {
                                pending_ref.fetch_add(num_children, Ordering::Relaxed);
                                for child in children {
                                    queue_ref.push(child);
                                }
                            }

                            local_sum += accumulator_ref(&item);
                            pending_ref.fetch_sub(1, Ordering::Release);
                        } else {
                            if pending_ref.load(Ordering::Acquire) == 0 {
                                break;
                            }
                            for _ in 0..8 {
                                core::hint::spin_loop();
                            }
                        }
                    }

                    result_ref.fetch_add(local_sum, Ordering::Relaxed);
                });
            }
        });

        result.load(Ordering::Relaxed)
    }
}