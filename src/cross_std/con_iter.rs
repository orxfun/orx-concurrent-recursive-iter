use crate::cross_std::queue::Queue;
use crate::cross_std::{
    chunk_puller::DynChunkPuller, dyn_seq_queue::DynSeqCrossbeam, local_worker::LocalWorker,
};
use crate::new_con_iter::NewConcurrentIter;
use crossbeam_deque::{Steal, Stealer, Worker};
use orx_concurrent_iter::ConcurrentIter;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Recursive concurrent iterator backed by crossbeam injector + work stealing.
pub struct ConcurrentRecursiveIterCrossbeam<I, E>
where
    I: IntoIterator,
    I::IntoIter: ExactSizeIterator,
    I::Item: Send,
    E: Fn(&I::Item) -> I + Send + Sync,
{
    pub(super) injector: Arc<Queue<I::Item>>,
    pub(super) extend: Arc<E>,
    pub(super) locals: Arc<Vec<LocalWorker<I::Item>>>,
    pub(super) stealers: Arc<Vec<Stealer<I::Item>>>,
    pub(super) pending: Arc<AtomicUsize>,
    pub(super) popped: Arc<AtomicUsize>,
    pub(super) stopped: Arc<AtomicBool>,
    exact_len: Option<usize>,
}

impl<I, E> ConcurrentRecursiveIterCrossbeam<I, E>
where
    I: IntoIterator,
    I::IntoIter: ExactSizeIterator,
    I::Item: Send,
    E: Fn(&I::Item) -> I + Send + Sync,
{
    fn with_locals_count(
        initial_elements: impl IntoIterator<Item = I::Item>,
        extend: E,
        num_locals: usize,
    ) -> Self {
        let injector = Arc::new(Queue::new());
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

    /// Creates a new crossbeam-backed recursive concurrent iterator.
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

    pub(super) fn decrement_pending(pending: &AtomicUsize) {
        let _ = pending.fetch_update(Ordering::AcqRel, Ordering::Relaxed, |x| x.checked_sub(1));
    }

    pub(super) fn owner_local<'a>(
        locals: &'a [LocalWorker<I::Item>],
        thread_idx: usize,
    ) -> &'a Worker<I::Item> {
        let local_idx = thread_idx % locals.len();
        locals[local_idx].as_owner_worker()
    }

    fn steal_one_from_injector_or_others(
        injector: &Queue<I::Item>,
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
        injector: &Queue<I::Item>,
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

    pub(super) fn next_impl(
        injector: &Queue<I::Item>,
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

        let children = extend(&item).into_iter();
        if children.len() > 0 && !stopped.load(Ordering::Acquire) {
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

    pub(super) fn pull_batch_into_impl(
        injector: &Queue<I::Item>,
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

            let children = extend(&item).into_iter();
            if children.len() > 0 && !stopped.load(Ordering::Acquire) {
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
}

impl<I, E> NewConcurrentIter for ConcurrentRecursiveIterCrossbeam<I, E>
where
    I: IntoIterator,
    I::IntoIter: ExactSizeIterator,
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
        DynChunkPuller {
            iter: self,
            chunk_size,
            thread_idx: Some(thread_idx),
            chunk_buffer: Vec::new(),
        }
    }
}

impl<I, E> ConcurrentIter for ConcurrentRecursiveIterCrossbeam<I, E>
where
    I: IntoIterator,
    I::IntoIter: ExactSizeIterator,
    I::Item: Send,
    E: Fn(&I::Item) -> I + Send + Sync,
{
    type Item = I::Item;

    type SequentialIter = DynSeqCrossbeam<I, E>;

    type ChunkPuller<'i>
        = DynChunkPuller<'i, I, E>
    where
        Self: 'i;

    fn into_seq_iter(self) -> Self::SequentialIter {
        DynSeqCrossbeam {
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
        match self.stopped.load(Ordering::Acquire) {
            true => (0, Some(0)),
            false => match self.exact_len {
                Some(exact_len) => {
                    let popped = self.popped.load(Ordering::Relaxed);
                    let remaining = exact_len.saturating_sub(popped);
                    (remaining, Some(remaining))
                }
                None => {
                    let pending = self.pending.load(Ordering::Acquire);
                    match pending {
                        0 => (0, Some(0)),
                        n => (n, None),
                    }
                }
            },
        }
    }

    fn is_completed_when_none_returned(&self) -> bool {
        self.stopped.load(Ordering::Acquire) || self.pending.load(Ordering::Acquire) == 0
    }

    fn chunk_puller(&self, chunk_size: usize) -> Self::ChunkPuller<'_> {
        DynChunkPuller {
            iter: self,
            chunk_size,
            thread_idx: None,
            chunk_buffer: Vec::new(),
        }
    }
}
