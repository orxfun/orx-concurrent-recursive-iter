use orx_concurrent_queue::ConcurrentQueue;
use orx_pinned_vec::{ConcurrentPinnedVec, IntoConcurrentPinnedVec};

pub struct Queue<'q, T, P>
where
    T: Send,
    P: ConcurrentPinnedVec<T>,
    <P as ConcurrentPinnedVec<T>>::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
{
    queue: &'q ConcurrentQueue<T, P>,
}

impl<'q, T, P> Queue<'q, T, P>
where
    T: Send,
    P: ConcurrentPinnedVec<T>,
    <P as ConcurrentPinnedVec<T>>::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
{
    #[inline(always)]
    pub fn push(&self, element: T) {
        self.queue.push(element);
    }

    #[inline(always)]
    pub fn extend<I>(&self, elements: I)
    where
        I: IntoIterator<Item = T>,
        I::IntoIter: ExactSizeIterator,
    {
        self.queue.extend(elements);
    }
}
