use crate::orx_queue::{chunk::DynChunk, queue::Queue};
use orx_concurrent_iter::ChunkPuller;
use orx_concurrent_queue::ConcurrentQueue;
use orx_pinned_vec::ConcurrentPinnedVec;

pub struct DynChunkPuller<'a, T, E, P>
where
    T: Send,
    E: Fn(&T, &Queue<T, P>) + Sync,
    P: ConcurrentPinnedVec<T>,
{
    extend: &'a E,
    queue: &'a ConcurrentQueue<T, P>,
    chunk_size: usize,
}

impl<'a, T, E, P> DynChunkPuller<'a, T, E, P>
where
    T: Send,
    E: Fn(&T, &Queue<T, P>) + Sync,
    P: ConcurrentPinnedVec<T>,
{
    pub(super) fn new(extend: &'a E, queue: &'a ConcurrentQueue<T, P>, chunk_size: usize) -> Self {
        Self {
            extend,
            queue,
            chunk_size,
        }
    }
}

impl<'a, T, E, P> ChunkPuller for DynChunkPuller<'a, T, E, P>
where
    T: Send,
    E: Fn(&T, &Queue<T, P>) + Sync,
    P: ConcurrentPinnedVec<T>,
{
    type ChunkItem = T;

    type Chunk<'c>
        = DynChunk<'c, T, E, P>
    where
        Self: 'c;

    #[inline(always)]
    fn chunk_size(&self) -> usize {
        self.chunk_size
    }

    fn resize_for_chunk_size(&mut self, new_chunk_size: usize) {
        self.chunk_size = new_chunk_size;
    }

    fn pull(&mut self) -> Option<Self::Chunk<'_>> {
        let chunk = self.queue.pull(self.chunk_size)?;
        Some(DynChunk::new(chunk, self.extend, self.queue.into()))
    }

    fn pull_with_idx(&mut self) -> Option<(usize, Self::Chunk<'_>)> {
        let (begin_idx, chunk) = self.queue.pull_with_idx(self.chunk_size)?;
        Some((
            begin_idx,
            DynChunk::new(chunk, self.extend, self.queue.into()),
        ))
    }
}
