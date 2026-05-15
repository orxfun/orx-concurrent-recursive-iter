use crate::concurrent_recursive_iter_shards2::{
    backend::ShardedQueue, chunk_puller::DynChunkPuller, dyn_seq_queue::DynSeqQueue, queue::Queue,
};
use core::sync::atomic::Ordering;
use orx_concurrent_iter::ConcurrentIter;
use orx_concurrent_queue::{ConcurrentQueue, DefaultConPinnedVec};
use orx_pinned_vec::{ConcurrentPinnedVec, IntoConcurrentPinnedVec};

pub struct ConcurrentRecursiveIter<T, E, P = DefaultConPinnedVec<T>>
where
    T: Send,
    P: ConcurrentPinnedVec<T>,
    <P as ConcurrentPinnedVec<T>>::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
    E: Fn(&T, &Queue<T, P>) + Sync,
{
    queue: ShardedQueue<T, P>,
    extend: E,
    exact_len: Option<usize>,
}

impl<T, E, P> From<(ConcurrentQueue<T, P>, E)> for ConcurrentRecursiveIter<T, E, P>
where
    T: Send,
    P: ConcurrentPinnedVec<T>,
    <P as ConcurrentPinnedVec<T>>::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
    E: Fn(&T, &Queue<T, P>) + Sync,
{
    fn from((queue, extend): (ConcurrentQueue<T, P>, E)) -> Self {
        Self {
            queue: ShardedQueue::from_single(queue),
            extend,
            exact_len: None,
        }
    }
}

impl<T, E, P> From<(ConcurrentQueue<T, P>, E, usize)> for ConcurrentRecursiveIter<T, E, P>
where
    T: Send,
    P: ConcurrentPinnedVec<T>,
    <P as ConcurrentPinnedVec<T>>::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
    E: Fn(&T, &Queue<T, P>) + Sync,
{
    fn from((queue, extend, exact_len): (ConcurrentQueue<T, P>, E, usize)) -> Self {
        Self {
            queue: ShardedQueue::from_single(queue),
            extend,
            exact_len: Some(exact_len),
        }
    }
}

impl<T, E> ConcurrentRecursiveIter<T, E, DefaultConPinnedVec<T>>
where
    T: Send,
    E: Fn(&T, &Queue<T, DefaultConPinnedVec<T>>) + Sync,
{
    pub fn new(initial_elements: impl IntoIterator<Item = T>, extend: E) -> Self {
        Self::new_with_shards(initial_elements, extend, 1)
    }

    pub fn new_with_shards(
        initial_elements: impl IntoIterator<Item = T>,
        extend: E,
        num_shards: usize,
    ) -> Self {
        assert!(
            num_shards == 1 || num_shards == 2,
            "concurrent_recursive_iter_shards2 supports only 1 or 2 shards"
        );

        let queue = match num_shards {
            1 => ShardedQueue::from_single(ConcurrentQueue::new()),
            2 => ShardedQueue::from_pair(ConcurrentQueue::new(), ConcurrentQueue::new()),
            _ => unreachable!(),
        };

        for element in initial_elements {
            queue.push(element);
        }

        Self {
            queue,
            extend,
            exact_len: None,
        }
    }

    pub fn new_exact(
        initial_elements: impl IntoIterator<Item = T>,
        extend: E,
        exact_len: usize,
    ) -> Self {
        Self::new_exact_with_shards(initial_elements, extend, exact_len, 1)
    }

    pub fn new_exact_with_shards(
        initial_elements: impl IntoIterator<Item = T>,
        extend: E,
        exact_len: usize,
        num_shards: usize,
    ) -> Self {
        assert!(
            num_shards == 1 || num_shards == 2,
            "concurrent_recursive_iter_shards2 supports only 1 or 2 shards"
        );

        let queue = match num_shards {
            1 => ShardedQueue::from_single(ConcurrentQueue::new()),
            2 => ShardedQueue::from_pair(ConcurrentQueue::new(), ConcurrentQueue::new()),
            _ => unreachable!(),
        };

        for element in initial_elements {
            queue.push(element);
        }

        Self {
            queue,
            extend,
            exact_len: Some(exact_len),
        }
    }
}

impl<T, E, P> ConcurrentIter for ConcurrentRecursiveIter<T, E, P>
where
    T: Send,
    P: ConcurrentPinnedVec<T>,
    <P as ConcurrentPinnedVec<T>>::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
    E: Fn(&T, &Queue<T, P>) + Sync,
{
    type Item = T;

    type SequentialIter = DynSeqQueue<T, P, E>;

    type ChunkPuller<'i>
        = DynChunkPuller<'i, T, E, P>
    where
        Self: 'i;

    fn into_seq_iter(self) -> Self::SequentialIter {
        DynSeqQueue::new(self.queue, self.extend)
    }

    fn skip_to_end(&self) {
        self.queue.skip_to_end();
    }

    fn next(&self) -> Option<Self::Item> {
        loop {
            if let Some((shard_idx, n)) = self.queue.pop() {
                (self.extend)(&n, &Queue::with_shard(&self.queue, shard_idx));
                return Some(n);
            }

            if self.queue.is_completed_when_none_returned() {
                return None;
            }

            core::hint::spin_loop();
        }
    }

    fn next_with_idx(&self) -> Option<(usize, Self::Item)> {
        loop {
            if let Some((idx, shard_idx, n)) = self.queue.pop_with_idx() {
                (self.extend)(&n, &Queue::with_shard(&self.queue, shard_idx));
                return Some((idx, n));
            }

            if self.queue.is_completed_when_none_returned() {
                return None;
            }

            core::hint::spin_loop();
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self.exact_len {
            Some(exact_len) => {
                let popped = self.queue.num_popped(Ordering::Relaxed);
                let remaining = exact_len - popped;
                (remaining, Some(remaining))
            }
            None => match self.queue.len() {
                0 => (0, Some(0)),
                n => (n, None),
            },
        }
    }

    fn is_completed_when_none_returned(&self) -> bool {
        self.queue.is_completed_when_none_returned()
    }

    fn chunk_puller(&self, chunk_size: usize) -> Self::ChunkPuller<'_> {
        DynChunkPuller::new(&self.extend, &self.queue, chunk_size)
    }
}
