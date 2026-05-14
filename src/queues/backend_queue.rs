#[doc(hidden)]
pub trait BackendQueue<T>: Send + Sync
where
    T: Send,
{
    type PullIter: ExactSizeIterator<Item = T>;

    fn push(&self, element: T);

    fn extend<I, Iter>(&self, elements: I)
    where
        I: IntoIterator<Item = T, IntoIter = Iter>,
        Iter: ExactSizeIterator<Item = T>;

    fn pop(&self) -> Option<T>;

    fn pull(&self, chunk_size: usize) -> Option<Self::PullIter>;

    fn pull_with_idx(&self, chunk_size: usize) -> Option<(usize, Self::PullIter)>;

    fn next_yield_idx(&self) -> usize;

    fn complete_one_extension(&self);

    fn complete_extension_batch_without_calling_extend(&self, remaining: usize);

    fn close_and_drain(&self);

    fn popped(&self) -> usize;

    fn queued(&self) -> usize;

    fn is_completed_when_none_returned(&self) -> bool;
}
