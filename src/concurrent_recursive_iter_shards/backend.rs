use core::sync::atomic::{AtomicUsize, Ordering};
use orx_concurrent_queue::{ConcurrentQueue, iter::QueueIterOwned};
use orx_pinned_vec::{ConcurrentPinnedVec, PinnedVec};
use orx_split_vec::SplitVec;

pub(super) struct ShardedQueue<T, P>
where
    T: Send,
    P: ConcurrentPinnedVec<T>,
{
    shards: SplitVec<ConcurrentQueue<T, P>>,
    push_cursor: AtomicUsize,
    pull_cursor: AtomicUsize,
    yielded_cursor: AtomicUsize,
}

impl<T, P> ShardedQueue<T, P>
where
    T: Send,
    P: ConcurrentPinnedVec<T>,
{
    pub(super) fn from_single(queue: ConcurrentQueue<T, P>) -> Self {
        let mut shards = SplitVec::with_doubling_growth_and_max_concurrent_capacity();
        shards.push(queue);
        Self::from_shards(shards)
    }

    pub(super) fn from_shards(shards: SplitVec<ConcurrentQueue<T, P>>) -> Self {
        let num_shards = shards.len().max(1);
        Self {
            shards,
            push_cursor: AtomicUsize::new(0),
            pull_cursor: AtomicUsize::new(0),
            yielded_cursor: AtomicUsize::new(0),
        }
        .with_non_empty_shards(num_shards)
    }

    fn with_non_empty_shards(self, num_shards: usize) -> Self {
        if self.shards.is_empty() {
            panic!("ShardedQueue requires at least one shard; got {num_shards}");
        }
        self
    }

    #[inline(always)]
    pub(super) fn num_shards(&self) -> usize {
        self.shards.len()
    }

    #[inline(always)]
    fn normalized_shard_idx(&self, shard_idx: usize) -> usize {
        shard_idx % self.num_shards().max(1)
    }

    #[inline(always)]
    fn next_push_shard(&self) -> usize {
        self.push_cursor.fetch_add(1, Ordering::Relaxed) % self.num_shards().max(1)
    }

    #[inline(always)]
    pub(super) fn push(&self, element: T) {
        let shard_idx = self.next_push_shard();
        self.shards[shard_idx].push(element);
    }

    #[inline(always)]
    pub(super) fn push_to_shard(&self, shard_idx: usize, element: T) {
        let shard_idx = self.normalized_shard_idx(shard_idx);
        self.shards[shard_idx].push(element);
    }

    #[inline(always)]
    pub(super) fn extend<I>(&self, elements: I)
    where
        I: IntoIterator<Item = T>,
        I::IntoIter: ExactSizeIterator,
    {
        let shard_idx = self.next_push_shard();
        self.shards[shard_idx].extend(elements);
    }

    #[inline(always)]
    pub(super) fn extend_to_shard<I>(&self, shard_idx: usize, elements: I)
    where
        I: IntoIterator<Item = T>,
        I::IntoIter: ExactSizeIterator,
    {
        let shard_idx = self.normalized_shard_idx(shard_idx);
        self.shards[shard_idx].extend(elements);
    }

    pub(super) fn pop(&self) -> Option<(usize, T)> {
        let num_shards = self.num_shards();
        let start = self.pull_cursor.fetch_add(1, Ordering::Relaxed) % num_shards;

        for offset in 0..num_shards {
            let shard_idx = (start + offset) % num_shards;
            if let Some(value) = self.shards[shard_idx].pop() {
                return Some((shard_idx, value));
            }
        }

        None
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

        let num_shards = self.num_shards();
        let start = self.pull_cursor.fetch_add(1, Ordering::Relaxed) % num_shards;

        for offset in 0..num_shards {
            let shard_idx = (start + offset) % num_shards;
            if let Some(chunk) = self.shards[shard_idx].pull(chunk_size) {
                return Some((shard_idx, chunk));
            }
        }

        None
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
        let mut total = 0;
        for i in 0..self.shards.len() {
            total += self.shards[i].len();
        }
        total
    }

    #[inline(always)]
    pub(super) fn num_popped(&self, order: Ordering) -> usize {
        let mut total = 0;
        for i in 0..self.shards.len() {
            total += self.shards[i].num_popped(order);
        }
        total
    }

    #[inline(always)]
    pub(super) fn num_write_reserved(&self, order: Ordering) -> usize {
        let mut total = 0;
        for i in 0..self.shards.len() {
            total += self.shards[i].num_write_reserved(order);
        }
        total
    }

    pub(super) fn skip_to_end(&self) {
        for shard in &self.shards {
            let len = shard.num_write_reserved(Ordering::Acquire);
            let _remaining_to_drop = shard.pull(len);
        }
    }
}
