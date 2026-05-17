use crate::new_con_iter::NewConcurrentIter;
use core::cell::UnsafeCell;
use crossbeam_deque::{Injector, Steal, Stealer, Worker};
use orx_concurrent_iter::{ChunkPuller, ConcurrentIter};
use std::iter::FusedIterator;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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
pub struct Con1<I, E>
where
    I: IntoIterator,
    I::Item: Send,
    E: Fn(&I::Item) -> I + Send + Sync,
{
    injector: Arc<Injector<I::Item>>,
    extend: Arc<E>,
    locals: Arc<Vec<LocalWorker<I::Item>>>,
    stealers: Arc<Vec<Stealer<I::Item>>>,
    pending: Arc<AtomicUsize>,
    popped: Arc<AtomicUsize>,
    stopped: Arc<AtomicBool>,
    exact_len: Option<usize>,
}

pub struct SeqCon1<I, E>
where
    I: IntoIterator,
    I::Item: Send,
    E: Fn(&I::Item) -> I + Send + Sync,
{
    injector: Arc<Injector<I::Item>>,
    extend: Arc<E>,
    locals: Arc<Vec<LocalWorker<I::Item>>>,
    stealers: Arc<Vec<Stealer<I::Item>>>,
    pending: Arc<AtomicUsize>,
    popped: Arc<AtomicUsize>,
    stopped: Arc<AtomicBool>,
}

pub struct Con1ChunkPuller<'a, I, E>
where
    I: IntoIterator,
    I::Item: Send,
    E: Fn(&I::Item) -> I + Send + Sync,
{
    iter: &'a Con1<I, E>,
    chunk_size: usize,
    thread_idx: Option<usize>,
    chunk_buffer: Vec<I::Item>,
}

impl<I, E> Con1<I, E>
where
    I: IntoIterator,
    I::Item: Send,
    E: Fn(&I::Item) -> I + Send + Sync,
{
    fn decrement_pending(pending: &AtomicUsize) {
        let _ = pending.fetch_update(Ordering::AcqRel, Ordering::Relaxed, |x| x.checked_sub(1));
    }

    fn owner_local<'a>(
        locals: &'a [LocalWorker<I::Item>],
        thread_idx: usize,
    ) -> &'a Worker<I::Item> {
        let local_idx = thread_idx % locals.len();
        locals[local_idx].as_owner_worker()
    }

    fn steal_one_from_injector_or_others(
        injector: &Injector<I::Item>,
        owner_local: &Worker<I::Item>,
        stealers: &[Stealer<I::Item>],
        local_idx: usize,
    ) -> Option<I::Item> {
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

    fn steal_one_global(
        injector: &Injector<I::Item>,
        stealers: &[Stealer<I::Item>],
    ) -> Option<I::Item> {
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
        injector: &Injector<I::Item>,
        extend: &E,
        locals: &[LocalWorker<I::Item>],
        stealers: &[Stealer<I::Item>],
        pending: &AtomicUsize,
        popped: &AtomicUsize,
        stopped: &AtomicBool,
        thread_idx: Option<usize>,
    ) -> Option<(usize, I::Item)> {
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

        let children: Vec<I::Item> = extend(&item).into_iter().collect();
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

    fn pull_batch_into_impl(
        injector: &Injector<I::Item>,
        extend: &E,
        locals: &[LocalWorker<I::Item>],
        stealers: &[Stealer<I::Item>],
        pending: &AtomicUsize,
        popped: &AtomicUsize,
        stopped: &AtomicBool,
        thread_idx: Option<usize>,
        chunk_size: usize,
        chunk: &mut Vec<I::Item>,
    ) -> Option<usize> {
        if chunk_size == 0 || stopped.load(Ordering::Acquire) {
            return None;
        }

        let owner_local = thread_idx.map(|idx| Self::owner_local(locals, idx));
        let owner_local_idx = thread_idx.map(|idx| idx % locals.len());

        chunk.clear();
        if chunk.capacity() < chunk_size {
            chunk.reserve(chunk_size - chunk.capacity());
        }

        let mut begin_idx = None;

        for _ in 0..chunk_size {
            if stopped.load(Ordering::Acquire) {
                break;
            }

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

            let item = match item {
                Some(item) => item,
                None => break,
            };

            let idx = popped.fetch_add(1, Ordering::Relaxed);
            begin_idx.get_or_insert(idx);

            let children: Vec<I::Item> = extend(&item).into_iter().collect();
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
            chunk.push(item);
        }

        begin_idx
    }

    fn default_num_locals() -> usize {
        let p = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1);
        p.max(1)
    }

    fn with_locals_count(
        initial_elements: impl IntoIterator<Item = I::Item>,
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
        let locals_vec: Vec<LocalWorker<I::Item>> =
            (0..locals_count).map(|_| LocalWorker::new_fifo()).collect();

        let stealers_vec: Vec<Stealer<I::Item>> =
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
    pub fn new(initial_elements: impl IntoIterator<Item = I::Item>, extend: E) -> Self {
        Self::with_locals_count(initial_elements, extend, Self::default_num_locals())
    }

    /// Creates a new iterator with explicit local worker shard count.
    pub fn new_with_locals(
        initial_elements: impl IntoIterator<Item = I::Item>,
        extend: E,
        num_locals: usize,
    ) -> Self {
        Self::with_locals_count(initial_elements, extend, num_locals)
    }

    /// Creates a new iterator with known exact output length.
    pub fn new_exact(
        initial_elements: impl IntoIterator<Item = I::Item>,
        extend: E,
        exact_len: usize,
    ) -> Self {
        let mut iter = Self::new(initial_elements, extend);
        iter.exact_len = Some(exact_len);
        iter
    }
}

impl<I, E> Iterator for SeqCon1<I, E>
where
    I: IntoIterator,
    I::Item: Send,
    E: Fn(&I::Item) -> I + Send + Sync,
{
    type Item = I::Item;

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

impl<I, E> FusedIterator for SeqCon1<I, E>
where
    I: IntoIterator,
    I::Item: Send,
    E: Fn(&I::Item) -> I + Send + Sync,
{
}

impl<'a, I, E> ChunkPuller for Con1ChunkPuller<'a, I, E>
where
    I: IntoIterator,
    I::Item: Send,
    E: Fn(&I::Item) -> I + Send + Sync,
{
    type ChunkItem = I::Item;

    type Chunk<'c>
        = std::vec::Drain<'c, I::Item>
    where
        Self: 'c;

    fn chunk_size(&self) -> usize {
        self.chunk_size
    }

    fn pull(&mut self) -> Option<Self::Chunk<'_>> {
        Con1::pull_batch_into_impl(
            &self.iter.injector,
            &*self.iter.extend,
            &self.iter.locals,
            &self.iter.stealers,
            &self.iter.pending,
            &self.iter.popped,
            &self.iter.stopped,
            self.thread_idx,
            self.chunk_size,
            &mut self.chunk_buffer,
        )?;

        Some(self.chunk_buffer.drain(..))
    }

    fn pull_with_idx(&mut self) -> Option<(usize, Self::Chunk<'_>)> {
        let begin_idx = Con1::pull_batch_into_impl(
            &self.iter.injector,
            &*self.iter.extend,
            &self.iter.locals,
            &self.iter.stealers,
            &self.iter.pending,
            &self.iter.popped,
            &self.iter.stopped,
            self.thread_idx,
            self.chunk_size.max(1),
            &mut self.chunk_buffer,
        )?;

        Some((begin_idx, self.chunk_buffer.drain(..)))
    }
}

impl<I, E> NewConcurrentIter for Con1<I, E>
where
    I: IntoIterator,
    I::Item: Send,
    E: Fn(&I::Item) -> I + Send + Sync,
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
            chunk_buffer: Vec::new(),
        }
    }
}

impl<I, E> ConcurrentIter for Con1<I, E>
where
    I: IntoIterator,
    I::Item: Send,
    E: Fn(&I::Item) -> I + Send + Sync,
{
    type Item = I::Item;

    type SequentialIter = SeqCon1<I, E>;

    type ChunkPuller<'i>
        = Con1ChunkPuller<'i, I, E>
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
            chunk_buffer: Vec::new(),
        }
    }
}
