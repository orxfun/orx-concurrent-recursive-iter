use crate::queue::Queue;
use orx_concurrent_queue::iter::QueueIterOwned;
use orx_pinned_vec::ConcurrentPinnedVec;

pub struct DynChunk<'a, T, E, P>
where
    T: Send,
    E: Fn(&T, &Queue<T, P>) + Sync,
    P: ConcurrentPinnedVec<T>,
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
{
    #[inline(always)]
    fn len(&self) -> usize {
        self.chunk.len()
    }
}
