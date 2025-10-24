use crate::{chunk_puller::DynChunkPuller, dyn_seq_queue::DynSeqQueue, queue::Queue};
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
    queue: ConcurrentQueue<T, P>,
    extend: E,
    exact_len: Option<usize>,
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
        let len = self.queue.num_write_reserved(Ordering::Acquire);
        let _remaining_to_drop = self.queue.pull(len);
    }

    fn next(&self) -> Option<Self::Item> {
        let n = self.queue.pop()?;
        (self.extend)(&n, &Queue::from(&self.queue));
        Some(n)
    }

    fn next_with_idx(&self) -> Option<(usize, Self::Item)> {
        let (idx, n) = self.queue.pop_with_idx()?;
        (self.extend)(&n, &Queue::from(&self.queue));
        Some((idx, n))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self.exact_len {
            Some(exact_len) => {
                let popped = self.queue.num_popped(Ordering::Relaxed);
                let remaining = exact_len - popped;
                (remaining, Some(remaining))
            }
            None => {
                let min = self.queue.len();
                match min {
                    0 => (0, Some(0)),
                    n => (n, None),
                }
            }
        }
    }

    fn is_completed_when_none_returned(&self) -> bool {
        let popped = self.queue.num_popped(Ordering::Relaxed);
        let write_reserved = self.queue.num_write_reserved(Ordering::Relaxed);
        popped >= write_reserved
    }

    fn chunk_puller(&self, chunk_size: usize) -> Self::ChunkPuller<'_> {
        DynChunkPuller::new(&self.extend, &self.queue, chunk_size)
    }
}
