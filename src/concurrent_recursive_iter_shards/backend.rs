use core::sync::atomic::{AtomicUsize, Ordering};
use orx_concurrent_queue::{ConcurrentQueue, iter::QueueIterOwned};
use orx_pinned_vec::{ConcurrentPinnedVec, IntoConcurrentPinnedVec};
use orx_pseudo_default::PseudoDefault;

const PREFERRED_SHARD_SLOTS: usize = 64;
const INVALID_SHARD: usize = usize::MAX;
const STEAL_PROBE_LIMIT: usize = 8;

pub(super) struct ShardedQueue<T, P>
where
    T: Send,
    P: ConcurrentPinnedVec<T>,
    P::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
{
    num_shards: usize,
    shards: [ConcurrentQueue<T, P>; 32],
    push_cursor: AtomicUsize,
    preferred_shard_by_slot: [AtomicUsize; PREFERRED_SHARD_SLOTS],
    yielded_cursor: AtomicUsize,
}

impl<T, P> ShardedQueue<T, P>
where
    T: Send,
    P: ConcurrentPinnedVec<T>,
    P::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
{
    pub(super) fn from_single(queue: ConcurrentQueue<T, P>) -> Self {
        let mut shards = [(); 32].map(|_| ConcurrentQueue::pseudo_default());
        shards[0] = queue;
        Self::from_shards(1, shards)
    }

    pub(super) fn from_shards(num_shards: usize, shards: [ConcurrentQueue<T, P>; 32]) -> Self {
        assert!(
            !shards.is_empty(),
            "ShardedQueue requires at least one shard"
        );

        Self {
            num_shards,
            shards,
            push_cursor: AtomicUsize::new(0),
            preferred_shard_by_slot: [(); PREFERRED_SHARD_SLOTS]
                .map(|_| AtomicUsize::new(INVALID_SHARD)),
            yielded_cursor: AtomicUsize::new(0),
        }
    }

    #[inline(always)]
    pub(super) fn num_shards(&self) -> usize {
        self.num_shards
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
    fn mix(x: usize) -> usize {
        let mut x = x as u64;
        x ^= x >> 33;
        x = x.wrapping_mul(0xff51afd7ed558ccd_u64);
        x ^= x >> 33;
        x = x.wrapping_mul(0xc4ceb9fe1a85ec53_u64);
        (x ^ (x >> 33)) as usize
    }

    #[inline(always)]
    fn current_thread_slot(&self) -> usize {
        let marker = 0usize;
        let addr = core::ptr::addr_of!(marker) as usize;
        Self::mix(addr) & (PREFERRED_SHARD_SLOTS - 1)
    }

    #[inline(always)]
    fn preferred_start(&self, slot: usize, num_shards: usize) -> usize {
        let preferred = self.preferred_shard_by_slot[slot].load(Ordering::Relaxed);
        if preferred < num_shards {
            preferred
        } else {
            Self::mix(slot) % num_shards
        }
    }

    #[inline(always)]
    fn store_preferred_shard(&self, slot: usize, shard_idx: usize) {
        self.preferred_shard_by_slot[slot].store(shard_idx, Ordering::Relaxed);
    }

    #[inline(always)]
    fn steal_probe_limit(num_shards: usize) -> usize {
        core::cmp::min(STEAL_PROBE_LIMIT, num_shards)
    }

    #[inline(always)]
    fn pop_with_start(
        &self,
        start: usize,
        num_shards: usize,
        probe_limit: usize,
    ) -> Option<(usize, T)> {
        for offset in 0..probe_limit {
            let shard_idx = (start + offset) % num_shards;
            if let Some(value) = self.shards[shard_idx].pop() {
                return Some((shard_idx, value));
            }
        }

        for offset in probe_limit..num_shards {
            let shard_idx = (start + offset) % num_shards;
            if let Some(value) = self.shards[shard_idx].pop() {
                return Some((shard_idx, value));
            }
        }

        None
    }

    #[inline(always)]
    fn pull_with_start(
        &self,
        start: usize,
        num_shards: usize,
        probe_limit: usize,
        chunk_size: usize,
    ) -> Option<(usize, QueueIterOwned<'_, T, P>)> {
        for offset in 0..probe_limit {
            let shard_idx = (start + offset) % num_shards;
            if let Some(chunk) = self.shards[shard_idx].pull(chunk_size) {
                return Some((shard_idx, chunk));
            }
        }

        for offset in probe_limit..num_shards {
            let shard_idx = (start + offset) % num_shards;
            if let Some(chunk) = self.shards[shard_idx].pull(chunk_size) {
                return Some((shard_idx, chunk));
            }
        }

        None
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
        let slot = self.current_thread_slot();
        let start = self.preferred_start(slot, num_shards);
        let probe_limit = Self::steal_probe_limit(num_shards);
        let result = self.pop_with_start(start, num_shards, probe_limit);
        if let Some((shard_idx, _)) = result.as_ref() {
            self.store_preferred_shard(slot, *shard_idx);
        }
        result
    }

    pub(super) fn pop_with_idx(&self) -> Option<(usize, usize, T)> {
        self.pop().map(|(shard_idx, value)| {
            let idx = self.yielded_cursor.fetch_add(1, Ordering::Relaxed);
            (idx, shard_idx, value)
        })
    }

    pub(super) fn pull(&self, chunk_size: usize) -> Option<(usize, QueueIterOwned<'_, T, P>)> {
        let num_shards = self.num_shards();
        let slot = self.current_thread_slot();
        let start = self.preferred_start(slot, num_shards);
        let probe_limit = Self::steal_probe_limit(num_shards);
        let result = self.pull_with_start(start, num_shards, probe_limit, chunk_size);
        if let Some((shard_idx, _)) = result.as_ref() {
            self.store_preferred_shard(slot, *shard_idx);
        }
        result
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
