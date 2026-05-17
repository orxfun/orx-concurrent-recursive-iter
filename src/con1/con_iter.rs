use core::cell::UnsafeCell;
use crossbeam_deque::{Injector, Steal, Stealer, Worker};
use orx_concurrent_iter::{ChunkPuller, ConcurrentIter};
use std::iter::FusedIterator;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::new_con_iter::NewConcurrentIter;

struct LocalWorker<T>
where
    T: Send,
{
    worker: UnsafeCell<Worker<T>>,
}

impl<T> LocalWorker<T>
where
    T: Send,
{
    fn new_fifo() -> Self {
        Self {
            worker: UnsafeCell::new(Worker::new_fifo()),
        }
    }

    fn stealer(&self) -> Stealer<T> {
        // SAFETY: `stealer` creation is read-only and can happen from any thread.
        unsafe { (&*self.worker.get()).stealer() }
    }

    fn as_owner_worker(&self) -> &Worker<T> {
        // SAFETY: owner-only access is enforced by thread-to-slot assignment.
        unsafe { &*self.worker.get() }
    }
}

// SAFETY: the underlying deque supports one owner thread and many stealers.
// Con1 assigns one owner per local slot and exposes only `Stealer` to other threads.
unsafe impl<T> Send for LocalWorker<T> where T: Send {}
// SAFETY: see `Send` rationale above.
unsafe impl<T> Sync for LocalWorker<T> where T: Send {}

/// Refined recursive concurrent iterator using crossbeam injector + work stealing.
///
/// This iterator is designed to be consumed through the generic [`ConcurrentIter`]
/// trait API while still leveraging local work queues and cross-thread stealing.
pub struct Con1<T, E>
where
    T: Send,
    E: Fn(&T) -> Vec<T> + Send + Sync,
{
    injector: Arc<Injector<T>>,
    extend: Arc<E>,
    locals: Arc<Vec<LocalWorker<T>>>,
    stealers: Arc<Vec<Stealer<T>>>,
    pending: Arc<AtomicUsize>,
    popped: Arc<AtomicUsize>,
    stopped: Arc<AtomicBool>,
    exact_len: Option<usize>,
}

pub struct SeqCon1<T, E>
where
    T: Send,
    E: Fn(&T) -> Vec<T> + Send + Sync,
{
    injector: Arc<Injector<T>>,
    extend: Arc<E>,
    locals: Arc<Vec<LocalWorker<T>>>,
    stealers: Arc<Vec<Stealer<T>>>,
    pending: Arc<AtomicUsize>,
    popped: Arc<AtomicUsize>,
    stopped: Arc<AtomicBool>,
}

pub struct Con1ChunkPuller<'a, T, E>
where
    T: Send,
    E: Fn(&T) -> Vec<T> + Send + Sync,
{
    iter: &'a Con1<T, E>,
    chunk_size: usize,
    thread_idx: Option<usize>,
}

impl<T, E> Con1<T, E>
where
    T: Send,
    E: Fn(&T) -> Vec<T> + Send + Sync,
{
    fn decrement_pending(pending: &AtomicUsize) {
        let _ = pending.fetch_update(Ordering::AcqRel, Ordering::Relaxed, |x| x.checked_sub(1));
    }

    fn owner_local<'a>(locals: &'a [LocalWorker<T>], thread_idx: usize) -> &'a Worker<T> {
        let local_idx = thread_idx % locals.len();
        locals[local_idx].as_owner_worker()
    }

    fn steal_one_from_injector_or_others(
        injector: &Injector<T>,
        owner_local: &Worker<T>,
        stealers: &[Stealer<T>],
        local_idx: usize,
    ) -> Option<T> {
        loop {
            let from_injector = injector.steal_batch_and_pop(owner_local);

            match from_injector {
                Steal::Success(item) => return Some(item),
                Steal::Retry => continue,
                Steal::Empty => {}
            }

            let mut saw_retry = false;
            for (idx, stealer) in stealers.iter().enumerate() {
                if idx == local_idx {
                    continue;
                }

                let stolen = stealer.steal_batch_and_pop(owner_local);

                match stolen {
                    Steal::Success(item) => return Some(item),
                    Steal::Retry => {
                        saw_retry = true;
                        break;
                    }
                    Steal::Empty => {}
                }
            }

            if saw_retry {
                continue;
            }

            return None;
        }
    }

    fn steal_one_global(injector: &Injector<T>, stealers: &[Stealer<T>]) -> Option<T> {
        loop {
            match injector.steal() {
                Steal::Success(item) => return Some(item),
                Steal::Retry => continue,
                Steal::Empty => {}
            }

            let mut saw_retry = false;
            for stealer in stealers {
                match stealer.steal() {
                    Steal::Success(item) => return Some(item),
                    Steal::Retry => {
                        saw_retry = true;
                        break;
                    }
                    Steal::Empty => {}
                }
            }

            if saw_retry {
                continue;
            }

            return None;
        }
    }

    fn next_impl(
        injector: &Injector<T>,
        extend: &E,
        locals: &[LocalWorker<T>],
        stealers: &[Stealer<T>],
        pending: &AtomicUsize,
        popped: &AtomicUsize,
        stopped: &AtomicBool,
        thread_idx: Option<usize>,
    ) -> Option<(usize, T)> {
        if stopped.load(Ordering::Acquire) {
            return None;
        }

        let owner_local = thread_idx.map(|idx| Self::owner_local(locals, idx));
        let owner_local_idx = thread_idx.map(|idx| idx % locals.len());

        let item = match owner_local {
            Some(local) => local.pop().or_else(|| {
                Self::steal_one_from_injector_or_others(
                    injector,
                    local,
                    stealers,
                    owner_local_idx.expect("owner local idx must exist"),
                )
            }),
            None => Self::steal_one_global(injector, stealers),
        };
        let item = item?;

        let idx = popped.fetch_add(1, Ordering::Relaxed);

        let children = extend(&item);
        if !children.is_empty() && !stopped.load(Ordering::Acquire) {
            pending.fetch_add(children.len(), Ordering::Relaxed);
            match owner_local {
                Some(local) => {
                    for child in children {
                        local.push(child);
                    }
                }
                None => {
                    for child in children {
                        injector.push(child);
                    }
                }
            }
        }

        Self::decrement_pending(pending);
        Some((idx, item))
    }

    fn default_num_locals() -> usize {
        let p = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1);
        p.max(1)
    }

    fn with_locals_count(
        initial_elements: impl IntoIterator<Item = T>,
        extend: E,
        num_locals: usize,
    ) -> Self {
        let injector = Arc::new(Injector::new());
        let mut pending = 0usize;
        for element in initial_elements {
            injector.push(element);
            pending += 1;
        }

        let locals_count = num_locals.max(1);
        let locals_vec: Vec<LocalWorker<T>> =
            (0..locals_count).map(|_| LocalWorker::new_fifo()).collect();

        let stealers_vec: Vec<Stealer<T>> =
            locals_vec.iter().map(|local| local.stealer()).collect();

        Self {
            injector,
            extend: Arc::new(extend),
            locals: Arc::new(locals_vec),
            stealers: Arc::new(stealers_vec),
            pending: Arc::new(AtomicUsize::new(pending)),
            popped: Arc::new(AtomicUsize::new(0)),
            stopped: Arc::new(AtomicBool::new(false)),
            exact_len: None,
        }
    }

    /// Creates a new refined crossbeam-backed recursive concurrent iterator.
    pub fn new(initial_elements: impl IntoIterator<Item = T>, extend: E) -> Self {
        Self::with_locals_count(initial_elements, extend, Self::default_num_locals())
    }

    /// Creates a new iterator with explicit local worker shard count.
    pub fn new_with_locals(
        initial_elements: impl IntoIterator<Item = T>,
        extend: E,
        num_locals: usize,
    ) -> Self {
        Self::with_locals_count(initial_elements, extend, num_locals)
    }

    /// Creates a new iterator with known exact output length.
    pub fn new_exact(
        initial_elements: impl IntoIterator<Item = T>,
        extend: E,
        exact_len: usize,
    ) -> Self {
        let mut iter = Self::new(initial_elements, extend);
        iter.exact_len = Some(exact_len);
        iter
    }
}

impl<T, E> Iterator for SeqCon1<T, E>
where
    T: Send,
    E: Fn(&T) -> Vec<T> + Send + Sync,
{
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        Con1::next_impl(
            &self.injector,
            &*self.extend,
            &self.locals,
            &self.stealers,
            &self.pending,
            &self.popped,
            &self.stopped,
            Some(0),
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

impl<T, E> FusedIterator for SeqCon1<T, E>
where
    T: Send,
    E: Fn(&T) -> Vec<T> + Send + Sync,
{
}

impl<'a, T, E> ChunkPuller for Con1ChunkPuller<'a, T, E>
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
        match self.thread_idx {
            Some(thread_idx) => {
                for _ in 0..self.chunk_size {
                    if let Some(item) =
                        NewConcurrentIter::next_with_thread_idx(self.iter, thread_idx)
                    {
                        chunk.push(item);
                    } else {
                        break;
                    }
                }
            }
            None => {
                for _ in 0..self.chunk_size {
                    if let Some(item) = ConcurrentIter::next(self.iter) {
                        chunk.push(item);
                    } else {
                        break;
                    }
                }
            }
        }

        match chunk.is_empty() {
            true => None,
            false => Some(chunk.into_iter()),
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

impl<T, E> NewConcurrentIter for Con1<T, E>
where
    T: Send,
    E: Fn(&T) -> Vec<T> + Send + Sync,
{
    fn next_with_thread_idx(&self, thread_idx: usize) -> Option<Self::Item> {
        Self::next_impl(
            &self.injector,
            &*self.extend,
            &self.locals,
            &self.stealers,
            &self.pending,
            &self.popped,
            &self.stopped,
            Some(thread_idx),
        )
        .map(|(_, x)| x)
    }

    fn chunk_puller_with_thread_idx(
        &self,
        chunk_size: usize,
        thread_idx: usize,
    ) -> Self::ChunkPuller<'_> {
        Con1ChunkPuller {
            iter: self,
            chunk_size,
            thread_idx: Some(thread_idx),
        }
    }
}

impl<T, E> ConcurrentIter for Con1<T, E>
where
    T: Send,
    E: Fn(&T) -> Vec<T> + Send + Sync,
{
    type Item = T;

    type SequentialIter = SeqCon1<T, E>;

    type ChunkPuller<'i>
        = Con1ChunkPuller<'i, T, E>
    where
        Self: 'i;

    fn into_seq_iter(self) -> Self::SequentialIter {
        SeqCon1 {
            injector: self.injector,
            extend: self.extend,
            locals: self.locals,
            stealers: self.stealers,
            pending: self.pending,
            popped: self.popped,
            stopped: self.stopped,
        }
    }

    fn skip_to_end(&self) {
        self.stopped.store(true, Ordering::Release);

        while self.injector.steal().is_success() {
            Self::decrement_pending(&self.pending);
        }
    }

    fn next(&self) -> Option<Self::Item> {
        Self::next_impl(
            &self.injector,
            &*self.extend,
            &self.locals,
            &self.stealers,
            &self.pending,
            &self.popped,
            &self.stopped,
            None,
        )
        .map(|(_, x)| x)
    }

    fn next_with_idx(&self) -> Option<(usize, Self::Item)> {
        Self::next_impl(
            &self.injector,
            &*self.extend,
            &self.locals,
            &self.stealers,
            &self.pending,
            &self.popped,
            &self.stopped,
            None,
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
        Con1ChunkPuller {
            iter: self,
            chunk_size,
            thread_idx: None,
        }
    }
}
