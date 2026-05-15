use crate::concurrent_recursive_iter_shards2::backend::ShardedQueue;
use orx_concurrent_queue::DefaultConPinnedVec;
use orx_pinned_vec::{ConcurrentPinnedVec, IntoConcurrentPinnedVec};

pub struct Queue<'a, T, P = DefaultConPinnedVec<T>>
where
    T: Send,
    P: ConcurrentPinnedVec<T>,
    P::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
{
    queue: &'a ShardedQueue<T, P>,
    preferred_shard: Option<usize>,
}

impl<T, P> Queue<'_, T, P>
where
    T: Send,
    P: ConcurrentPinnedVec<T>,
    P::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
{
    #[inline(always)]
    pub fn push(&self, element: T) {
        match self.preferred_shard {
            Some(shard_idx) => self.queue.push_to_shard(shard_idx, element),
            None => self.queue.push(element),
        }
    }

    #[inline(always)]
    pub fn extend<I>(&self, elements: I)
    where
        I: IntoIterator<Item = T>,
        I::IntoIter: ExactSizeIterator,
    {
        match self.preferred_shard {
            Some(shard_idx) => self.queue.extend_to_shard(shard_idx, elements),
            None => self.queue.extend(elements),
        }
    }
}

impl<'a, T, P> From<&'a ShardedQueue<T, P>> for Queue<'a, T, P>
where
    T: Send,
    P: ConcurrentPinnedVec<T>,
    P::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
{
    #[inline(always)]
    fn from(queue: &'a ShardedQueue<T, P>) -> Self {
        Self {
            queue,
            preferred_shard: None,
        }
    }
}

impl<'a, T, P> Queue<'a, T, P>
where
    T: Send,
    P: ConcurrentPinnedVec<T>,
    P::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
{
    #[inline(always)]
    pub(super) fn with_shard(queue: &'a ShardedQueue<T, P>, shard_idx: usize) -> Self {
        Self {
            queue,
            preferred_shard: Some(shard_idx),
        }
    }
}
