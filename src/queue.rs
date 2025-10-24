use orx_concurrent_queue::{ConcurrentQueue, DefaultConPinnedVec};
use orx_pinned_vec::ConcurrentPinnedVec;

pub struct Queue<'a, T, P = DefaultConPinnedVec<T>>
where
    T: Send,
    P: ConcurrentPinnedVec<T>,
{
    queue: &'a ConcurrentQueue<T, P>,
}

impl<T, P> Queue<'_, T, P>
where
    T: Send,
    P: ConcurrentPinnedVec<T>,
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

impl<'a, T, P> From<&'a ConcurrentQueue<T, P>> for Queue<'a, T, P>
where
    T: Send,
    P: ConcurrentPinnedVec<T>,
{
    #[inline(always)]
    fn from(queue: &'a ConcurrentQueue<T, P>) -> Self {
        Self { queue }
    }
}
