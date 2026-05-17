use crossbeam_deque::{Injector, Steal, Stealer, Worker};
use orx_concurrent_iter::{ChunkPuller, ConcurrentIter};
use std::iter::FusedIterator;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

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
    popped: Arc<AtomicUsize>,
    stopped: Arc<AtomicBool>,
    exact_len: Option<usize>,
}

pub struct SeqIterCross<T, E>
where
    T: Send,
    E: Fn(&T) -> Vec<T> + Send + Sync,
{
    injector: Arc<Injector<T>>,
    extend: Arc<E>,
    pending: Arc<AtomicUsize>,
    popped: Arc<AtomicUsize>,
    stopped: Arc<AtomicBool>,
}

pub struct CrossChunkPuller<'a, T, E>
where
    T: Send,
    E: Fn(&T) -> Vec<T> + Send + Sync,
{
    iter: &'a ConcurrentIterCross<T, E>,
    chunk_size: usize,
}

impl<T, E> ConcurrentIterCross<T, E>
where
    T: Send,
    E: Fn(&T) -> Vec<T> + Send + Sync,
{
    fn steal_one(injector: &Injector<T>) -> Option<T> {
        loop {
            match injector.steal() {
                Steal::Success(item) => return Some(item),
                Steal::Empty => return None,
                Steal::Retry => {}
            }
        }
    }

    fn decrement_pending(pending: &AtomicUsize) {
        let _ = pending.fetch_update(Ordering::AcqRel, Ordering::Relaxed, |x| x.checked_sub(1));
    }

    fn next_impl(
        injector: &Injector<T>,
        extend: &E,
        pending: &AtomicUsize,
        popped: &AtomicUsize,
        stopped: &AtomicBool,
    ) -> Option<(usize, T)> {
        if stopped.load(Ordering::Acquire) {
            return None;
        }

        let item = Self::steal_one(injector)?;
        let idx = popped.fetch_add(1, Ordering::Relaxed);

        let children = extend(&item);
        if !children.is_empty() && !stopped.load(Ordering::Acquire) {
            let num_children = children.len();
            pending.fetch_add(num_children, Ordering::Relaxed);
            // Batch push children to reduce contention
            for child in children {
                injector.push(child);
            }
        }

        Self::decrement_pending(pending);
        Some((idx, item))
    }

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
            popped: Arc::new(AtomicUsize::new(0)),
            stopped: Arc::new(AtomicBool::new(false)),
            exact_len: None,
        }
    }

    /// Create a new concurrent recursive iterator with exact length.
    pub fn new_exact(
        initial_elements: impl IntoIterator<Item = T>,
        extend: E,
        exact_len: usize,
    ) -> Self {
        let mut iter = Self::new(initial_elements, extend);
        iter.exact_len = Some(exact_len);
        iter
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
                let popped_ref = Arc::clone(&self.popped);
                let stopped_ref = Arc::clone(&self.stopped);
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
                            if stopped_ref.load(Ordering::Acquire) {
                                Self::decrement_pending(&pending_ref);
                                continue;
                            }

                            // Get children and push them to local queue
                            let children = extend_ref(&item);
                            let num_children = children.len();
                            if num_children > 0 && !stopped_ref.load(Ordering::Acquire) {
                                pending_ref.fetch_add(num_children, Ordering::Relaxed);
                                for child in children {
                                    local.push(child);
                                }
                            }

                            local_sum += accumulator_ref(&item);
                            popped_ref.fetch_add(1, Ordering::Relaxed);
                            Self::decrement_pending(&pending_ref);
                        } else {
                            if stopped_ref.load(Ordering::Acquire)
                                || pending_ref.load(Ordering::Acquire) == 0
                            {
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

impl<T, E> Iterator for SeqIterCross<T, E>
where
    T: Send,
    E: Fn(&T) -> Vec<T> + Send + Sync,
{
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        ConcurrentIterCross::next_impl(
            &self.injector,
            &*self.extend,
            &self.pending,
            &self.popped,
            &self.stopped,
        )
        .map(|(_, x)| x)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.stopped.load(Ordering::Acquire) || self.pending.load(Ordering::Acquire) == 0 {
            (0, Some(0))
        } else {
            (0, None)
        }
    }
}

impl<T, E> FusedIterator for SeqIterCross<T, E>
where
    T: Send,
    E: Fn(&T) -> Vec<T> + Send + Sync,
{
}

impl<'a, T, E> ChunkPuller for CrossChunkPuller<'a, T, E>
where
    T: Send,
    E: Fn(&T) -> Vec<T> + Send + Sync,
{
    type ChunkItem = T;

    type Chunk<'c>
        = std::vec::IntoIter<T>
    where
        Self: 'c;

    fn chunk_size(&self) -> usize {
        self.chunk_size
    }

    fn pull(&mut self) -> Option<Self::Chunk<'_>> {
        let mut chunk = Vec::with_capacity(self.chunk_size);

        for _ in 0..self.chunk_size {
            if let Some(item) = ConcurrentIter::next(self.iter) {
                chunk.push(item);
            } else {
                break;
            }
        }

        if chunk.is_empty() {
            None
        } else {
            Some(chunk.into_iter())
        }
    }

    fn pull_with_idx(&mut self) -> Option<(usize, Self::Chunk<'_>)> {
        let (begin_idx, first) = ConcurrentIter::next_with_idx(self.iter)?;
        let mut chunk = Vec::with_capacity(self.chunk_size);
        chunk.push(first);

        for _ in 1..self.chunk_size {
            if let Some(item) = ConcurrentIter::next(self.iter) {
                chunk.push(item);
            } else {
                break;
            }
        }

        Some((begin_idx, chunk.into_iter()))
    }
}

impl<T, E> ConcurrentIter for ConcurrentIterCross<T, E>
where
    T: Send,
    E: Fn(&T) -> Vec<T> + Send + Sync,
{
    type Item = T;

    type SequentialIter = SeqIterCross<T, E>;

    type ChunkPuller<'i>
        = CrossChunkPuller<'i, T, E>
    where
        Self: 'i;

    fn into_seq_iter(self) -> Self::SequentialIter {
        SeqIterCross {
            injector: self.injector,
            extend: self.extend,
            pending: self.pending,
            popped: self.popped,
            stopped: self.stopped,
        }
    }

    fn skip_to_end(&self) {
        self.stopped.store(true, Ordering::Release);
        while Self::steal_one(&self.injector).is_some() {
            Self::decrement_pending(&self.pending);
        }
    }

    fn next(&self) -> Option<Self::Item> {
        Self::next_impl(
            &self.injector,
            &*self.extend,
            &self.pending,
            &self.popped,
            &self.stopped,
        )
        .map(|(_, x)| x)
    }

    fn next_with_idx(&self) -> Option<(usize, Self::Item)> {
        Self::next_impl(
            &self.injector,
            &*self.extend,
            &self.pending,
            &self.popped,
            &self.stopped,
        )
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.stopped.load(Ordering::Acquire) {
            return (0, Some(0));
        }

        match self.exact_len {
            Some(exact_len) => {
                let popped = self.popped.load(Ordering::Relaxed);
                let remaining = exact_len.saturating_sub(popped);
                (remaining, Some(remaining))
            }
            None => {
                if self.pending.load(Ordering::Acquire) == 0 {
                    (0, Some(0))
                } else {
                    (0, None)
                }
            }
        }
    }

    fn is_completed_when_none_returned(&self) -> bool {
        self.stopped.load(Ordering::Acquire) || self.pending.load(Ordering::Acquire) == 0
    }

    fn chunk_puller(&self, chunk_size: usize) -> Self::ChunkPuller<'_> {
        CrossChunkPuller {
            iter: self,
            chunk_size,
        }
    }
}
