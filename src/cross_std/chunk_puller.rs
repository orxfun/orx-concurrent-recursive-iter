use crate::cross_std::{chunk::DynChunk, con_iter::ConcurrentRecursiveIterCrossbeam};
use orx_concurrent_iter::ChunkPuller;

pub struct DynChunkPuller<'a, I, E>
where
    I: IntoIterator,
    I::IntoIter: ExactSizeIterator,
    I::Item: Send,
    E: Fn(&I::Item) -> I + Send + Sync,
{
    pub(super) iter: &'a ConcurrentRecursiveIterCrossbeam<I, E>,
    pub(super) chunk_size: usize,
    pub(super) thread_idx: Option<usize>,
    pub(super) chunk_buffer: Vec<I::Item>,
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
        ConcurrentRecursiveIterCrossbeam::pull_batch_into_impl(
            &self.iter.injector,
            &*self.iter.extend,
            &self.iter.locals,
            &self.iter.stealers,
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
        let begin_idx = ConcurrentRecursiveIterCrossbeam::pull_batch_into_impl(
            &self.iter.injector,
            &*self.iter.extend,
            &self.iter.locals,
            &self.iter.stealers,
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