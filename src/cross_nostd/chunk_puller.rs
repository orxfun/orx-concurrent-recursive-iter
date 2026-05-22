use crate::cross_nostd::{chunk::DynChunk, con_iter::ConcurrentRecursiveIterCrossbeamNoStd};
use orx_concurrent_iter::ChunkPuller;

pub struct DynChunkPuller<'a, I, E>
where
    I: IntoIterator,
    I::IntoIter: ExactSizeIterator,
    I::Item: Send,
    E: Fn(&I::Item) -> I + Send + Sync,
{
    pub(super) iter: &'a ConcurrentRecursiveIterCrossbeamNoStd<I, E>,
    pub(super) chunk_size: usize,
    pub(super) thread_idx: Option<usize>,
    pub(super) chunk_buffer: alloc::vec::Vec<I::Item>,
}

impl<'a, I, E> ChunkPuller for DynChunkPuller<'a, I, E>
where
    I: IntoIterator,
    I::IntoIter: ExactSizeIterator,
    I::Item: Send,
    E: Fn(&I::Item) -> I + Send + Sync,
{
    type ChunkItem = I::Item;

    type Chunk<'c>
        = DynChunk<'c, I::Item>
    where
        Self: 'c;

    fn chunk_size(&self) -> usize {
        self.chunk_size
    }

    fn pull(&mut self) -> Option<Self::Chunk<'_>> {
        ConcurrentRecursiveIterCrossbeamNoStd::pull_batch_into_impl(
            &self.iter.queue,
            &*self.iter.extend,
            &self.iter.pending,
            &self.iter.popped,
            &self.iter.stopped,
            self.thread_idx,
            self.chunk_size,
            &mut self.chunk_buffer,
        )?;

        Some(self.chunk_buffer.drain(..))
    }

    fn pull_with_idx(&mut self) -> Option<(usize, Self::Chunk<'_>)> {
        let begin_idx = ConcurrentRecursiveIterCrossbeamNoStd::pull_batch_into_impl(
            &self.iter.queue,
            &*self.iter.extend,
            &self.iter.pending,
            &self.iter.popped,
            &self.iter.stopped,
            self.thread_idx,
            self.chunk_size.max(1),
            &mut self.chunk_buffer,
        )?;

        Some((begin_idx, self.chunk_buffer.drain(..)))
    }
}
