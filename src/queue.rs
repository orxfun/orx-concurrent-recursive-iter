use orx_concurrent_queue::{ConcurrentQueue, DefaultConPinnedVec};
use orx_pinned_vec::ConcurrentPinnedVec;

/// A queue of elements that will be returned by the [`ConcurrentRecursiveIter`].
///
/// Note that the concurrent recursive iterator is built on top of the [`ConcurrentQueue`].
/// `Queue` is just a wrapper over the concurrent queue, exposing only two methods:
///
/// * `push` to add one task at a time,
/// * `extend` to add multiple tasks with known length together with a single update.
///
/// Notice that `push` is very flexible and makes the pushed element available for iteration
/// as fast as possible.
///
/// On the other hand, `extend` minimizes the overhead of concurrency.
///
/// [`ConcurrentRecursiveIter`]: crate::ConcurrentRecursiveIter
/// [`ConcurrentQueue`]: orx_concurrent_queue::ConcurrentQueue
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
    /// Pushes the `element` to the iterator, making it available to all threads as fast as possible.
    #[inline(always)]
    pub fn push(&self, element: T) {
        self.queue.push(element);
    }

    /// Pushes all `elements` to the iterator with a single update on the concurrent state.
    ///
    /// All of the elements will be available to all threads at the same time, once writing all of them
    /// to the queue is completed.
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
