use crate::concurrent_recursive_iter_shards::{
    backend::ShardedQueue, chunk::DynChunk, queue::Queue,
};
use orx_concurrent_iter::ChunkPuller;
use orx_pinned_vec::{ConcurrentPinnedVec, IntoConcurrentPinnedVec};

pub struct DynChunkPuller<'a, T, E, P>
where
    T: Send,
    E: Fn(&T, &Queue<T, P>) + Sync,
    P: ConcurrentPinnedVec<T>,
    P::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
{
    extend: &'a E,
    queue: &'a ShardedQueue<T, P>,
    chunk_size: usize,
}

impl<'a, T, E, P> DynChunkPuller<'a, T, E, P>
where
    T: Send,
    E: Fn(&T, &Queue<T, P>) + Sync,
    P: ConcurrentPinnedVec<T>,
    P::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
{
    pub(super) fn new(extend: &'a E, queue: &'a ShardedQueue<T, P>, chunk_size: usize) -> Self {
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
    P::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
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

    fn pull(&mut self) -> Option<Self::Chunk<'_>> {
        let (shard_idx, chunk) = self.queue.pull(self.chunk_size)?;
        Some(DynChunk::new(
            chunk,
            self.extend,
            Queue::with_shard(self.queue, shard_idx),
        ))
    }

    fn pull_with_idx(&mut self) -> Option<(usize, Self::Chunk<'_>)> {
        let (begin_idx, shard_idx, chunk) = self.queue.pull_with_idx(self.chunk_size)?;
        Some((
            begin_idx,
            DynChunk::new(chunk, self.extend, Queue::with_shard(self.queue, shard_idx)),
        ))
    }
}
