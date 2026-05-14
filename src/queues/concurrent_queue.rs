use crate::queues::backend_queue::BackendQueue;
use orx_concurrent_queue::{ConcurrentQueue, iter::QueueIterOwned};
use orx_pinned_vec::ConcurrentPinnedVec;

impl<T, P> BackendQueue<T> for ConcurrentQueue<T, P>
where
    T: Send,
    P: ConcurrentPinnedVec<T>,
{
    type PullIter<'a>
        = QueueIterOwned<'a, T, P>
    where
        Self: 'a;

    fn push(&self, value: T) {
        ConcurrentQueue::push(self, value);
    }

    fn extend<I, Iter>(&self, values: I)
    where
        I: IntoIterator<Item = T, IntoIter = Iter>,
        Iter: ExactSizeIterator<Item = T>,
    {
        ConcurrentQueue::extend(self, values);
    }

    fn pop(&self) -> Option<T> {
        ConcurrentQueue::pop(self)
    }

    fn pull(&self, chunk_size: usize) -> Option<Self::PullIter<'_>> {
        ConcurrentQueue::pull(self, chunk_size)
    }

    fn pull_with_idx(&self, chunk_size: usize) -> Option<(usize, Self::PullIter<'_>)> {
        ConcurrentQueue::pull_with_idx(self, chunk_size)
    }

    fn next_yield_idx(&self) -> usize {
        todo!()
    }

    fn complete_one_extension(&self) {
        todo!()
    }

    fn complete_extension_batch_without_calling_extend(&self, remaining: usize) {
        todo!()
    }

    fn close_and_drain(&self) {
        todo!()
    }

    fn popped(&self) -> usize {
        todo!()
    }

    fn queued(&self) -> usize {
        todo!()
    }

    fn is_completed_when_none_returned(&self) -> bool {
        todo!()
    }
}
