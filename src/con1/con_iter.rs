use crossbeam_deque::{Injector, Steal, Stealer, Worker};
use orx_concurrent_iter::{ChunkPuller, ConcurrentIter};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::iter::FusedIterator;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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
    locals: Arc<Vec<Mutex<Worker<T>>>>,
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
    locals: Arc<Vec<Mutex<Worker<T>>>>,
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
}

impl<T, E> Con1<T, E>
where
    T: Send,
    E: Fn(&T) -> Vec<T> + Send + Sync,
{
    fn decrement_pending(pending: &AtomicUsize) {
        let _ = pending.fetch_update(Ordering::AcqRel, Ordering::Relaxed, |x| x.checked_sub(1));
    }

    fn choose_local_idx(num_locals: usize) -> usize {
        if num_locals <= 1 {
            return 0;
        }

        let tid = std::thread::current().id();
        let mut hasher = DefaultHasher::new();
        tid.hash(&mut hasher);
        (hasher.finish() as usize) % num_locals
    }

    fn steal_one_from_injector_or_others(
        injector: &Injector<T>,
        locals: &[Mutex<Worker<T>>],
        stealers: &[Stealer<T>],
        local_idx: usize,
    ) -> Option<T> {
        loop {
            let from_injector = {
                let local = locals[local_idx].lock().expect("local worker poisoned");
                injector.steal_batch_and_pop(&local)
            };

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

                let stolen = {
                    let local = locals[local_idx].lock().expect("local worker poisoned");
                    stealer.steal_batch_and_pop(&local)
                };

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

    fn next_impl(
        injector: &Injector<T>,
        extend: &E,
        locals: &[Mutex<Worker<T>>],
        stealers: &[Stealer<T>],
        pending: &AtomicUsize,
        popped: &AtomicUsize,
        stopped: &AtomicBool,
    ) -> Option<(usize, T)> {
        if stopped.load(Ordering::Acquire) {
            return None;
        }

        let local_idx = Self::choose_local_idx(locals.len());

        let item = {
            let local = locals[local_idx].lock().expect("local worker poisoned");
            local.pop()
        }
        .or_else(|| {
            Self::steal_one_from_injector_or_others(injector, locals, stealers, local_idx)
        })?;

        let idx = popped.fetch_add(1, Ordering::Relaxed);

        let children = extend(&item);
        if !children.is_empty() && !stopped.load(Ordering::Acquire) {
            pending.fetch_add(children.len(), Ordering::Relaxed);
            let local = &mut *locals[local_idx].lock().expect("local worker poisoned");
            for child in children {
                local.push(child);
            }
        }

        Self::decrement_pending(pending);
        Some((idx, item))
    }

    fn default_num_locals() -> usize {
        let p = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1);
        p.max(1) * 4
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
        let locals_vec: Vec<Mutex<Worker<T>>> = (0..locals_count)
            .map(|_| Mutex::new(Worker::new_fifo()))
            .collect();

        let stealers_vec: Vec<Stealer<T>> = locals_vec
            .iter()
            .map(|local| local.lock().expect("local worker poisoned").stealer())
            .collect();

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

        for local in self.locals.iter() {
            let local = &mut *local.lock().expect("local worker poisoned");
            while local.pop().is_some() {
                Self::decrement_pending(&self.pending);
            }
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
        }
    }
}
