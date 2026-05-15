use core::sync::atomic::{AtomicUsize, Ordering};
use orx_concurrent_queue::{ConcurrentQueue, iter::QueueIterOwned};
use orx_pinned_vec::{ConcurrentPinnedVec, IntoConcurrentPinnedVec};
use orx_pseudo_default::PseudoDefault;

pub(super) struct ShardedQueue<T, P>
where
    T: Send,
    P: ConcurrentPinnedVec<T>,
    P::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
{
    num_shards: usize,
    shard1: ConcurrentQueue<T, P>,
    shard2: ConcurrentQueue<T, P>,
    push_cursor: AtomicUsize,
    pull_cursor: AtomicUsize,
    yielded_cursor: AtomicUsize,
}

impl<T, P> ShardedQueue<T, P>
where
    T: Send,
    P: ConcurrentPinnedVec<T>,
    P::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
{
    pub(super) fn from_single(queue: ConcurrentQueue<T, P>) -> Self {
        Self {
            num_shards: 1,
            shard1: queue,
            shard2: ConcurrentQueue::pseudo_default(),
            push_cursor: AtomicUsize::new(0),
            pull_cursor: AtomicUsize::new(0),
            yielded_cursor: AtomicUsize::new(0),
        }
    }

    pub(super) fn from_pair(shard1: ConcurrentQueue<T, P>, shard2: ConcurrentQueue<T, P>) -> Self {
        Self {
            num_shards: 2,
            shard1,
            shard2,
            push_cursor: AtomicUsize::new(0),
            pull_cursor: AtomicUsize::new(0),
            yielded_cursor: AtomicUsize::new(0),
        }
    }

    #[inline(always)]
    fn num_shards(&self) -> usize {
        self.num_shards
    }

    #[inline(always)]
    fn shard(&self, shard_idx: usize) -> &ConcurrentQueue<T, P> {
        if shard_idx == 0 {
            &self.shard1
        } else {
            &self.shard2
        }
    }

    #[inline(always)]
    fn normalized_shard_idx(&self, shard_idx: usize) -> usize {
        shard_idx % self.num_shards()
    }

    #[inline(always)]
    fn next_push_shard(&self) -> usize {
        self.push_cursor.fetch_add(1, Ordering::Relaxed) % self.num_shards()
    }

    #[inline(always)]
    fn next_pull_shard(&self) -> usize {
        self.pull_cursor.fetch_add(1, Ordering::Relaxed) % self.num_shards()
    }

    #[inline(always)]
    pub(super) fn push(&self, element: T) {
        let shard_idx = self.next_push_shard();
        self.shard(shard_idx).push(element);
    }

    #[inline(always)]
    pub(super) fn push_to_shard(&self, shard_idx: usize, element: T) {
        let shard_idx = self.normalized_shard_idx(shard_idx);
        self.shard(shard_idx).push(element);
    }

    #[inline(always)]
    pub(super) fn extend<I>(&self, elements: I)
    where
        I: IntoIterator<Item = T>,
        I::IntoIter: ExactSizeIterator,
    {
        let shard_idx = self.next_push_shard();
        self.shard(shard_idx).extend(elements);
    }

    #[inline(always)]
    pub(super) fn extend_to_shard<I>(&self, shard_idx: usize, elements: I)
    where
        I: IntoIterator<Item = T>,
        I::IntoIter: ExactSizeIterator,
    {
        let shard_idx = self.normalized_shard_idx(shard_idx);
        self.shard(shard_idx).extend(elements);
    }

    pub(super) fn pop(&self) -> Option<(usize, T)> {
        // No work stealing: each attempt looks only at one chosen shard.
        let shard_idx = self.next_pull_shard();
        self.shard(shard_idx).pop().map(|value| (shard_idx, value))
    }

    pub(super) fn pop_with_idx(&self) -> Option<(usize, usize, T)> {
        self.pop().map(|(shard_idx, value)| {
            let idx = self.yielded_cursor.fetch_add(1, Ordering::Relaxed);
            (idx, shard_idx, value)
        })
    }

    pub(super) fn pull(&self, chunk_size: usize) -> Option<(usize, QueueIterOwned<'_, T, P>)> {
        if chunk_size == 0 {
            return None;
        }

        // No work stealing: each attempt looks only at one chosen shard.
        let shard_idx = self.next_pull_shard();
        self.shard(shard_idx)
            .pull(chunk_size)
            .map(|chunk| (shard_idx, chunk))
    }

    pub(super) fn pull_with_idx(
        &self,
        chunk_size: usize,
    ) -> Option<(usize, usize, QueueIterOwned<'_, T, P>)> {
        let (shard_idx, chunk) = self.pull(chunk_size)?;
        let begin_idx = self
            .yielded_cursor
            .fetch_add(chunk.len(), Ordering::Relaxed);
        Some((begin_idx, shard_idx, chunk))
    }

    #[inline(always)]
    pub(super) fn len(&self) -> usize {
        if self.num_shards == 1 {
            self.shard1.len()
        } else {
            self.shard1.len() + self.shard2.len()
        }
    }

    #[inline(always)]
    pub(super) fn num_popped(&self, order: Ordering) -> usize {
        if self.num_shards == 1 {
            self.shard1.num_popped(order)
        } else {
            self.shard1.num_popped(order) + self.shard2.num_popped(order)
        }
    }

    #[inline(always)]
    pub(super) fn num_write_reserved(&self, order: Ordering) -> usize {
        if self.num_shards == 1 {
            self.shard1.num_write_reserved(order)
        } else {
            self.shard1.num_write_reserved(order) + self.shard2.num_write_reserved(order)
        }
    }

    #[inline(always)]
    pub(super) fn is_completed_when_none_returned(&self) -> bool {
        let popped = self.num_popped(Ordering::Relaxed);
        let write_reserved = self.num_write_reserved(Ordering::Relaxed);
        popped >= write_reserved
    }

    pub(super) fn skip_to_end(&self) {
        let len1 = self.shard1.num_write_reserved(Ordering::Acquire);
        let _remaining_to_drop1 = self.shard1.pull(len1);

        if self.num_shards == 2 {
            let len2 = self.shard2.num_write_reserved(Ordering::Acquire);
            let _remaining_to_drop2 = self.shard2.pull(len2);
        }
    }
}
