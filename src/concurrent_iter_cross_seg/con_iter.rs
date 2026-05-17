use crossbeam_queue::SegQueue;
use orx_concurrent_iter::{ChunkPuller, ConcurrentIter};
use std::iter::FusedIterator;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

/// Concurrent recursive iterator using crossbeam's SegQueue.
pub struct ConcurrentIterCrossSeg<T, E>
where
    T: Send,
    E: Fn(&T) -> Vec<T> + Send + Sync,
{
    queue: Arc<SegQueue<T>>,
    extend: Arc<E>,
    pending: Arc<AtomicUsize>,
    popped: Arc<AtomicUsize>,
    stopped: Arc<AtomicBool>,
    exact_len: Option<usize>,
}

pub struct SeqIterCrossSeg<T, E>
where
    T: Send,
    E: Fn(&T) -> Vec<T> + Send + Sync,
{
    queue: Arc<SegQueue<T>>,
    extend: Arc<E>,
    pending: Arc<AtomicUsize>,
    popped: Arc<AtomicUsize>,
    stopped: Arc<AtomicBool>,
}

pub struct CrossSegChunkPuller<'a, T, E>
where
    T: Send,
    E: Fn(&T) -> Vec<T> + Send + Sync,
{
    iter: &'a ConcurrentIterCrossSeg<T, E>,
    chunk_size: usize,
}

impl<T, E> ConcurrentIterCrossSeg<T, E>
where
    T: Send,
    E: Fn(&T) -> Vec<T> + Send + Sync,
{
    fn decrement_pending(pending: &AtomicUsize) {
        let _ = pending.fetch_update(Ordering::AcqRel, Ordering::Relaxed, |x| x.checked_sub(1));
    }

    fn next_impl(
        queue: &SegQueue<T>,
        extend: &E,
        pending: &AtomicUsize,
        popped: &AtomicUsize,
        stopped: &AtomicBool,
    ) -> Option<(usize, T)> {
        if stopped.load(Ordering::Acquire) {
            return None;
        }

        let item = queue.pop()?;
        let idx = popped.fetch_add(1, Ordering::Relaxed);

        let children = extend(&item);
        if !children.is_empty() && !stopped.load(Ordering::Acquire) {
            pending.fetch_add(children.len(), Ordering::Relaxed);
            for child in children {
                queue.push(child);
            }
        }

        Self::decrement_pending(pending);
        Some((idx, item))
    }

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

        std::thread::scope(|scope| {
            for _ in 0..num_threads.max(1) {
                let queue_ref = Arc::clone(&self.queue);
                let pending_ref = Arc::clone(&self.pending);
                let popped_ref = Arc::clone(&self.popped);
                let stopped_ref = Arc::clone(&self.stopped);
                let extend_ref = Arc::clone(&self.extend);
                let result_ref = Arc::clone(&result);
                let accumulator_ref = Arc::clone(&accumulator);

                scope.spawn(move || {
                    let mut local_sum = 0u64;

                    loop {
                        if let Some(item) = queue_ref.pop() {
                            if stopped_ref.load(Ordering::Acquire) {
                                Self::decrement_pending(&pending_ref);
                                continue;
                            }

                            let children = extend_ref(&item);
                            let num_children = children.len();
                            if num_children > 0 && !stopped_ref.load(Ordering::Acquire) {
                                pending_ref.fetch_add(num_children, Ordering::Relaxed);
                                for child in children {
                                    queue_ref.push(child);
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

impl<T, E> Iterator for SeqIterCrossSeg<T, E>
where
    T: Send,
    E: Fn(&T) -> Vec<T> + Send + Sync,
{
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        ConcurrentIterCrossSeg::next_impl(
            &self.queue,
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

impl<T, E> FusedIterator for SeqIterCrossSeg<T, E>
where
    T: Send,
    E: Fn(&T) -> Vec<T> + Send + Sync,
{
}

impl<'a, T, E> ChunkPuller for CrossSegChunkPuller<'a, T, E>
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

impl<T, E> ConcurrentIter for ConcurrentIterCrossSeg<T, E>
where
    T: Send,
    E: Fn(&T) -> Vec<T> + Send + Sync,
{
    type Item = T;

    type SequentialIter = SeqIterCrossSeg<T, E>;

    type ChunkPuller<'i>
        = CrossSegChunkPuller<'i, T, E>
    where
        Self: 'i;

    fn into_seq_iter(self) -> Self::SequentialIter {
        SeqIterCrossSeg {
            queue: self.queue,
            extend: self.extend,
            pending: self.pending,
            popped: self.popped,
            stopped: self.stopped,
        }
    }

    fn skip_to_end(&self) {
        self.stopped.store(true, Ordering::Release);
        while self.queue.pop().is_some() {
            Self::decrement_pending(&self.pending);
        }
    }

    fn next(&self) -> Option<Self::Item> {
        Self::next_impl(
            &self.queue,
            &*self.extend,
            &self.pending,
            &self.popped,
            &self.stopped,
        )
        .map(|(_, x)| x)
    }

    fn next_with_idx(&self) -> Option<(usize, Self::Item)> {
        Self::next_impl(
            &self.queue,
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
        CrossSegChunkPuller {
            iter: self,
            chunk_size,
        }
    }
}
