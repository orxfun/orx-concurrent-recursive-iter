use orx_concurrent_queue::ConcurrentQueue;
use orx_pinned_vec::ConcurrentPinnedVec;

pub struct Queue<'q, T, P>
where
    T: Send,
    P: ConcurrentPinnedVec<T>,
{
    queue: &'q ConcurrentQueue<T, P>,
}

impl<'q, T, P> Queue<'q, T, P>
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

impl<'q, T, P> From<&'q ConcurrentQueue<T, P>> for Queue<'q, T, P>
where
    T: Send,
    P: ConcurrentPinnedVec<T>,
{
    #[inline(always)]
    fn from(queue: &'q ConcurrentQueue<T, P>) -> Self {
        Self { queue }
    }
}
