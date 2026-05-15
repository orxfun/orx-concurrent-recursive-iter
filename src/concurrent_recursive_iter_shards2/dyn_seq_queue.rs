use crate::concurrent_recursive_iter_shards2::{backend::ShardedQueue, queue::Queue};
use core::iter::FusedIterator;
use orx_pinned_vec::{ConcurrentPinnedVec, IntoConcurrentPinnedVec};

pub struct DynSeqQueue<T, P, E>
where
    T: Send,
    P: ConcurrentPinnedVec<T>,
    <P as ConcurrentPinnedVec<T>>::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
    E: Fn(&T, &Queue<T, P>) + Sync,
{
    queue: ShardedQueue<T, P>,
    extend: E,
}

impl<T, P, E> DynSeqQueue<T, P, E>
where
    T: Send,
    P: ConcurrentPinnedVec<T>,
    <P as ConcurrentPinnedVec<T>>::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
    E: Fn(&T, &Queue<T, P>) + Sync,
{
    pub(super) fn new(queue: ShardedQueue<T, P>, extend: E) -> Self {
        Self { queue, extend }
    }
}

impl<T, P, E> Iterator for DynSeqQueue<T, P, E>
where
    T: Send,
    P: ConcurrentPinnedVec<T>,
    <P as ConcurrentPinnedVec<T>>::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
    E: Fn(&T, &Queue<T, P>) + Sync,
{
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some((shard_idx, element)) = self.queue.pop() {
                (self.extend)(&element, &Queue::with_shard(&self.queue, shard_idx));
                return Some(element);
            }

            if self.queue.is_completed_when_none_returned() {
                return None;
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self.queue.len() {
            0 => (0, Some(0)),
            n => (n, None),
        }
    }
}

impl<T, P, E> FusedIterator for DynSeqQueue<T, P, E>
where
    T: Send,
    P: ConcurrentPinnedVec<T>,
    <P as ConcurrentPinnedVec<T>>::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
    E: Fn(&T, &Queue<T, P>) + Sync,
{
}
