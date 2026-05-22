use crate::cross_no_std::{
    chunk_puller::DynChunkPuller, dyn_seq_queue::DynSeqCrossbeamNoStd, queue::Queue,
};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use orx_concurrent_iter::ConcurrentIter;

/// Recursive concurrent iterator using crossbeam SegQueue (no-std friendly internals).
pub struct ConcurrentRecursiveIterCrossbeamNoStd<I, E>
where
    I: IntoIterator,
    I::IntoIter: ExactSizeIterator,
    I::Item: Send,
    E: Fn(&I::Item) -> I + Send + Sync,
{
    pub(super) queue: Arc<Queue<I::Item>>,
    pub(super) extend: Arc<E>,
    pub(super) pending: Arc<AtomicUsize>,
    pub(super) popped: Arc<AtomicUsize>,
    pub(super) stopped: Arc<AtomicBool>,
    exact_len: Option<usize>,
}

impl<I, E> ConcurrentRecursiveIterCrossbeamNoStd<I, E>
where
    I: IntoIterator,
    I::IntoIter: ExactSizeIterator,
    I::Item: Send,
    E: Fn(&I::Item) -> I + Send + Sync,
{
    /// Creates a new SegQueue-backed recursive concurrent iterator.
    pub fn new(
        initial_elements: impl IntoIterator<Item = I::Item>,
        extend: E,
        exact_len: Option<usize>,
        _num_locals: Option<usize>,
    ) -> Self {
        let queue = Arc::new(Queue::new());
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
            exact_len,
        }
    }

    fn decrement_pending(pending: &AtomicUsize) {
        let _ = pending.fetch_update(Ordering::AcqRel, Ordering::Relaxed, |x| x.checked_sub(1));
    }

    pub(super) fn next_impl(
        queue: &Queue<I::Item>,
        extend: &E,
        pending: &AtomicUsize,
        popped: &AtomicUsize,
        stopped: &AtomicBool,
        _thread_idx: Option<usize>,
    ) -> Option<(usize, I::Item)> {
        if stopped.load(Ordering::Acquire) {
            return None;
        }

        let item = queue.pop()?;
        let idx = popped.fetch_add(1, Ordering::Relaxed);

        let children = extend(&item).into_iter();
        if children.len() > 0 && !stopped.load(Ordering::Acquire) {
            pending.fetch_add(children.len(), Ordering::Relaxed);
            for child in children {
                queue.push(child);
            }
        }

        Self::decrement_pending(pending);
        Some((idx, item))
    }

    pub(super) fn pull_batch_into_impl(
        queue: &Queue<I::Item>,
        extend: &E,
        pending: &AtomicUsize,
        popped: &AtomicUsize,
        stopped: &AtomicBool,
        _thread_idx: Option<usize>,
        chunk_size: usize,
        chunk: &mut Vec<I::Item>,
    ) -> Option<usize> {
        debug_assert!(chunk.is_empty());
        if chunk_size == 0 || stopped.load(Ordering::Acquire) {
            return None;
        }

        chunk.clear();
        if chunk.capacity() < chunk_size {
            chunk.reserve(chunk_size - chunk.capacity());
        }

        let mut begin_idx = None;

        for _ in 0..chunk_size {
            if stopped.load(Ordering::Acquire) {
                break;
            }

            let item = match queue.pop() {
                Some(item) => item,
                None => break,
            };

            let idx = popped.fetch_add(1, Ordering::Relaxed);
            begin_idx.get_or_insert(idx);

            let children = extend(&item).into_iter();
            if children.len() > 0 && !stopped.load(Ordering::Acquire) {
                pending.fetch_add(children.len(), Ordering::Relaxed);
                for child in children {
                    queue.push(child);
                }
            }

            Self::decrement_pending(pending);
            chunk.push(item);
        }

        begin_idx
    }
}

impl<I, E> ConcurrentIter for ConcurrentRecursiveIterCrossbeamNoStd<I, E>
where
    I: IntoIterator,
    I::IntoIter: ExactSizeIterator,
    I::Item: Send,
    E: Fn(&I::Item) -> I + Send + Sync,
{
    type Item = I::Item;

    type SequentialIter = DynSeqCrossbeamNoStd<I, E>;

    type ChunkPuller<'i>
        = DynChunkPuller<'i, I, E>
    where
        Self: 'i;

    fn into_seq_iter(self) -> Self::SequentialIter {
        DynSeqCrossbeamNoStd {
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
            None,
        )
        .map(|(_, x)| x)
    }

    fn next_by(&self, thread_idx: usize) -> Option<Self::Item> {
        Self::next_impl(
            &self.queue,
            &*self.extend,
            &self.pending,
            &self.popped,
            &self.stopped,
            Some(thread_idx),
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
            None,
        )
    }

    fn next_with_idx_by(&self, thread_idx: usize) -> Option<(usize, Self::Item)> {
        Self::next_impl(
            &self.queue,
            &*self.extend,
            &self.pending,
            &self.popped,
            &self.stopped,
            Some(thread_idx),
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
            chunk_buffer: Default::default(),
        }
    }

    fn chunk_puller_by(&self, chunk_size: usize, thread_idx: usize) -> Self::ChunkPuller<'_> {
        DynChunkPuller {
            iter: self,
            chunk_size,
            thread_idx: Some(thread_idx),
            chunk_buffer: Default::default(),
        }
    }
}
