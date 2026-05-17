use orx_concurrent_iter::ConcurrentIter;

pub trait NewConcurrentIter: ConcurrentIter {
    /// Similar to `next` but receives the index of the thread calling next.
    /// Note that if the computation is carried out with 4 threads,
    /// then the `thread_idx` will be either one of the integers 0, 1, 2 or 3.
    fn next_with_thread_idx(&self, thread_idx: usize) -> Option<Self::Item>;

    /// Similar to `chunk_puller` but receives the index of the thread creating the chunk puller.
    /// Note that if the computation is carried out with 4 threads,
    /// then the `thread_idx` will be either one of the integers 0, 1, 2 or 3.
    fn chunk_puller_with_thread_idx(&self, thread_idx: usize) -> Self::ChunkPuller<'_>;
}
