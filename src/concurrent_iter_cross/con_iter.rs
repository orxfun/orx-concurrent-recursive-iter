use crossbeam_deque::{Injector, Steal, Stealer, Worker};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Concurrent recursive iterator using crossbeam's work-stealing deque.
///
/// This provides a way to process recursive data structures concurrently
/// using work-stealing to balance load across threads.
pub struct ConcurrentIterCross<T, E>
where
    T: Send,
    E: Fn(&T) -> Vec<T> + Send + Sync,
{
    injector: Arc<Injector<T>>,
    extend: Arc<E>,
    pending: Arc<AtomicUsize>,
}

impl<T, E> ConcurrentIterCross<T, E>
where
    T: Send,
    E: Fn(&T) -> Vec<T> + Send + Sync,
{
    /// Create a new concurrent recursive iterator using crossbeam's work-stealing deque.
    pub fn new(initial_elements: impl IntoIterator<Item = T>, extend: E) -> Self {
        let injector = Arc::new(Injector::new());
        let mut pending = 0usize;
        for element in initial_elements {
            injector.push(element);
            pending += 1;
        }

        Self {
            injector,
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

        let locals: Vec<Worker<T>> = (0..num_threads.max(1))
            .map(|_| Worker::new_fifo())
            .collect();
        let stealers: Vec<Stealer<T>> = locals.iter().map(Worker::stealer).collect();

        std::thread::scope(|scope| {
            for (tid, local) in locals.into_iter().enumerate() {
                let injector_ref = Arc::clone(&self.injector);
                let pending_ref = Arc::clone(&self.pending);
                let extend_ref = Arc::clone(&self.extend);
                let result_ref = Arc::clone(&result);
                let stealers_ref = stealers.clone();
                let accumulator_ref = Arc::clone(&accumulator);

                scope.spawn(move || {
                    let mut local_sum = 0u64;

                    loop {
                        let mut task = local.pop();

                        if task.is_none() {
                            match injector_ref.steal_batch_and_pop(&local) {
                                Steal::Success(x) => task = Some(x),
                                Steal::Retry => continue,
                                Steal::Empty => {
                                    for (j, stealer) in stealers_ref.iter().enumerate() {
                                        if j == tid {
                                            continue;
                                        }
                                        match stealer.steal_batch_and_pop(&local) {
                                            Steal::Success(x) => {
                                                task = Some(x);
                                                break;
                                            }
                                            Steal::Retry => {
                                                task = None;
                                                break;
                                            }
                                            Steal::Empty => {}
                                        }
                                    }
                                }
                            }
                        }

                        if let Some(item) = task {
                            // Get children and push them to local queue
                            let children = extend_ref(&item);
                            let num_children = children.len();
                            if num_children > 0 {
                                pending_ref.fetch_add(num_children, Ordering::Relaxed);
                                for child in children {
                                    local.push(child);
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
