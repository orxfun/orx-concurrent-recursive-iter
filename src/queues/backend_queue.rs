#[doc(hidden)]
pub trait BackendQueue<T>: Sync
where
    T: Send,
{
    type PullIter<'a>: ExactSizeIterator<Item = T>
    where
        Self: 'a;

    fn push(&self, value: T);

    fn extend<I, Iter>(&self, values: I)
    where
        I: IntoIterator<Item = T, IntoIter = Iter>,
        Iter: ExactSizeIterator<Item = T>;

    fn pop(&self) -> Option<T>;

    fn pull(&self, chunk_size: usize) -> Option<Self::PullIter<'_>>;

    fn pull_with_idx(&self, chunk_size: usize) -> Option<(usize, Self::PullIter<'_>)>;

    fn next_yield_idx(&self) -> usize;

    fn complete_one_extension(&self);

    fn complete_extension_batch_without_calling_extend(&self, remaining: usize);

    fn close_and_drain(&self);

    fn popped(&self) -> usize;

    fn queued(&self) -> usize;

    fn is_completed_when_none_returned(&self) -> bool;
}
