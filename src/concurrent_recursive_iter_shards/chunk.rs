use crate::concurrent_recursive_iter_shards::queue::Queue;
use orx_concurrent_queue::iter::QueueIterOwned;
use orx_pinned_vec::{ConcurrentPinnedVec, IntoConcurrentPinnedVec};

pub struct DynChunk<'a, T, E, P>
where
    T: Send,
    E: Fn(&T, &Queue<T, P>) + Sync,
    P: ConcurrentPinnedVec<T>,
    P::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
{
    chunk: QueueIterOwned<'a, T, P>,
    extend: &'a E,
    queue: Queue<'a, T, P>,
}

impl<'a, T, E, P> DynChunk<'a, T, E, P>
where
    T: Send,
    E: Fn(&T, &Queue<T, P>) + Sync,
    P: ConcurrentPinnedVec<T>,
    P::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
{
    pub(super) fn new(
        chunk: QueueIterOwned<'a, T, P>,
        extend: &'a E,
        queue: Queue<'a, T, P>,
    ) -> Self {
        Self {
            chunk,
            extend,
            queue,
        }
    }
}

impl<'a, T, E, P> Iterator for DynChunk<'a, T, E, P>
where
    T: Send,
    E: Fn(&T, &Queue<T, P>) + Sync,
    P: ConcurrentPinnedVec<T>,
    P::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
{
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        let n = self.chunk.next()?;
        (self.extend)(&n, &self.queue);
        // let children = (self.extend)(&n);
        // self.queue.extend(children);
        Some(n)
    }

    #[inline(always)]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.chunk.len();
        (len, Some(len))
    }
}

impl<'a, T, E, P> ExactSizeIterator for DynChunk<'a, T, E, P>
where
    T: Send,
    E: Fn(&T, &Queue<T, P>) + Sync,
    P: ConcurrentPinnedVec<T>,
    P::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
{
    #[inline(always)]
    fn len(&self) -> usize {
        self.chunk.len()
    }
}
