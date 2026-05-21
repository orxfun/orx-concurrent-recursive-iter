use crate::new_con_iter::NewConcurrentIter;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::iter::FusedIterator;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use crossbeam_queue::SegQueue;
use orx_concurrent_iter::{ChunkPuller, ConcurrentIter};

/// Recursive concurrent iterator using crossbeam SegQueue.
pub struct Con2<I, E>
where
    I: IntoIterator,
    I::IntoIter: ExactSizeIterator,
    I::Item: Send,
    E: Fn(&I::Item) -> I + Send + Sync,
{
    queue: Arc<SegQueue<I::Item>>,
    extend: Arc<E>,
    pending: Arc<AtomicUsize>,
    popped: Arc<AtomicUsize>,
    stopped: Arc<AtomicBool>,
    exact_len: Option<usize>,
}

pub struct SeqCon2<I, E>
where
    I: IntoIterator,
    I::IntoIter: ExactSizeIterator,
    I::Item: Send,
    E: Fn(&I::Item) -> I + Send + Sync,
{
    queue: Arc<SegQueue<I::Item>>,
    extend: Arc<E>,
    pending: Arc<AtomicUsize>,
    popped: Arc<AtomicUsize>,
    stopped: Arc<AtomicBool>,
}

pub struct Con2ChunkPuller<'a, I, E>
where
    I: IntoIterator,
    I::IntoIter: ExactSizeIterator,
    I::Item: Send,
    E: Fn(&I::Item) -> I + Send + Sync,
{
    iter: &'a Con2<I, E>,
    chunk_size: usize,
    thread_idx: Option<usize>,
    chunk_buffer: Vec<I::Item>,
}

impl<I, E> Con2<I, E>
where
    I: IntoIterator,
    I::IntoIter: ExactSizeIterator,
    I::Item: Send,
    E: Fn(&I::Item) -> I + Send + Sync,
{
    fn decrement_pending(pending: &AtomicUsize) {
        let _ = pending.fetch_update(Ordering::AcqRel, Ordering::Relaxed, |x| x.checked_sub(1));
    }

    fn next_impl(
        queue: &SegQueue<I::Item>,
        extend: &E,
        pending: &AtomicUsize,
        popped: &AtomicUsize,
        stopped: &AtomicBool,
        _thread_idx: Option<usize>,
    ) -> Option<(usize, I::Item)> {
        if stopped.load(Ordering::Acquire) {
            return None;
        }

        let item = queue.pop()?;
        let idx = popped.fetch_add(1, Ordering::Relaxed);

        let children = extend(&item).into_iter();
        if children.len() > 0 && !stopped.load(Ordering::Acquire) {
            pending.fetch_add(children.len(), Ordering::Relaxed);
            for child in children {
                queue.push(child);
            }
        }

        Self::decrement_pending(pending);
        Some((idx, item))
    }

    fn pull_batch_into_impl(
        queue: &SegQueue<I::Item>,
        extend: &E,
        pending: &AtomicUsize,
        popped: &AtomicUsize,
        stopped: &AtomicBool,
        _thread_idx: Option<usize>,
        chunk_size: usize,
        chunk: &mut Vec<I::Item>,
    ) -> Option<usize> {
        if chunk_size == 0 || stopped.load(Ordering::Acquire) {
            return None;
        }

        chunk.clear();
        if chunk.capacity() < chunk_size {
            chunk.reserve(chunk_size - chunk.capacity());
        }

        let mut begin_idx = None;

        for _ in 0..chunk_size {
            if stopped.load(Ordering::Acquire) {
                break;
            }

            let item = match queue.pop() {
                Some(item) => item,
                None => break,
            };

            let idx = popped.fetch_add(1, Ordering::Relaxed);
            begin_idx.get_or_insert(idx);

            let children = extend(&item).into_iter();
            if children.len() > 0 && !stopped.load(Ordering::Acquire) {
                pending.fetch_add(children.len(), Ordering::Relaxed);
                for child in children {
                    queue.push(child);
                }
            }

            Self::decrement_pending(pending);
            chunk.push(item);
        }

        begin_idx
    }

    /// Creates a new SegQueue-backed recursive concurrent iterator.
    pub fn new(initial_elements: impl IntoIterator<Item = I::Item>, extend: E) -> Self {
        let queue = Arc::new(SegQueue::new());
        let mut pending = 0usize;
        for element in initial_elements {
            queue.push(element);
            pending += 1;
        }

        Self {
            queue,
            extend: Arc::new(extend),
            pending: Arc::new(AtomicUsize::new(pending)),
            popped: Arc::new(AtomicUsize::new(0)),
            stopped: Arc::new(AtomicBool::new(false)),
            exact_len: None,
        }
    }

    /// Creates a new iterator with known exact output length.
    pub fn new_exact(
        initial_elements: impl IntoIterator<Item = I::Item>,
        extend: E,
        exact_len: usize,
    ) -> Self {
        let mut iter = Self::new(initial_elements, extend);
        iter.exact_len = Some(exact_len);
        iter
    }
}

impl<I, E> Iterator for SeqCon2<I, E>
where
    I: IntoIterator,
    I::IntoIter: ExactSizeIterator,
    I::Item: Send,
    E: Fn(&I::Item) -> I + Send + Sync,
{
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        Con2::next_impl(
            &self.queue,
            &*self.extend,
            &self.pending,
            &self.popped,
            &self.stopped,
            Some(0),
        )
        .map(|(_, x)| x)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.stopped.load(Ordering::Acquire) || self.pending.load(Ordering::Acquire) == 0 {
            (0, Some(0))
        } else {
            (0, None)
        }
    }
}

impl<I, E> FusedIterator for SeqCon2<I, E>
where
    I: IntoIterator,
    I::IntoIter: ExactSizeIterator,
    I::Item: Send,
    E: Fn(&I::Item) -> I + Send + Sync,
{
}

impl<'a, I, E> ChunkPuller for Con2ChunkPuller<'a, I, E>
where
    I: IntoIterator,
    I::IntoIter: ExactSizeIterator,
    I::Item: Send,
    E: Fn(&I::Item) -> I + Send + Sync,
{
    type ChunkItem = I::Item;

    type Chunk<'c>
        = alloc::vec::Drain<'c, I::Item>
    where
        Self: 'c;

    fn chunk_size(&self) -> usize {
        self.chunk_size
    }

    fn pull(&mut self) -> Option<Self::Chunk<'_>> {
        Con2::pull_batch_into_impl(
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
        let begin_idx = Con2::pull_batch_into_impl(
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

impl<I, E> NewConcurrentIter for Con2<I, E>
where
    I: IntoIterator,
    I::IntoIter: ExactSizeIterator,
    I::Item: Send,
    E: Fn(&I::Item) -> I + Send + Sync,
{
    fn next_with_thread_idx(&self, thread_idx: usize) -> Option<Self::Item> {
        Self::next_impl(
            &self.queue,
            &*self.extend,
            &self.pending,
            &self.popped,
            &self.stopped,
            Some(thread_idx),
        )
        .map(|(_, x)| x)
    }

    fn chunk_puller_with_thread_idx(
        &self,
        chunk_size: usize,
        thread_idx: usize,
    ) -> Self::ChunkPuller<'_> {
        Con2ChunkPuller {
            iter: self,
            chunk_size,
            thread_idx: Some(thread_idx),
            chunk_buffer: Vec::new(),
        }
    }
}

impl<I, E> ConcurrentIter for Con2<I, E>
where
    I: IntoIterator,
    I::IntoIter: ExactSizeIterator,
    I::Item: Send,
    E: Fn(&I::Item) -> I + Send + Sync,
{
    type Item = I::Item;

    type SequentialIter = SeqCon2<I, E>;

    type ChunkPuller<'i>
        = Con2ChunkPuller<'i, I, E>
    where
        Self: 'i;

    fn into_seq_iter(self) -> Self::SequentialIter {
        SeqCon2 {
            queue: self.queue,
            extend: self.extend,
            pending: self.pending,
            popped: self.popped,
            stopped: self.stopped,
        }
    }

    fn skip_to_end(&self) {
        self.stopped.store(true, Ordering::Release);
        while self.queue.pop().is_some() {
            Self::decrement_pending(&self.pending);
        }
    }

    fn next(&self) -> Option<Self::Item> {
        Self::next_impl(
            &self.queue,
            &*self.extend,
            &self.pending,
            &self.popped,
            &self.stopped,
            None,
        )
        .map(|(_, x)| x)
    }

    fn next_with_idx(&self) -> Option<(usize, Self::Item)> {
        Self::next_impl(
            &self.queue,
            &*self.extend,
            &self.pending,
            &self.popped,
            &self.stopped,
            None,
        )
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.stopped.load(Ordering::Acquire) {
            return (0, Some(0));
        }

        match self.exact_len {
            Some(exact_len) => {
                let popped = self.popped.load(Ordering::Relaxed);
                let remaining = exact_len.saturating_sub(popped);
                (remaining, Some(remaining))
            }
            None => {
                if self.pending.load(Ordering::Acquire) == 0 {
                    (0, Some(0))
                } else {
                    (0, None)
                }
            }
        }
    }

    fn is_completed_when_none_returned(&self) -> bool {
        self.stopped.load(Ordering::Acquire) || self.pending.load(Ordering::Acquire) == 0
    }

    fn chunk_puller(&self, chunk_size: usize) -> Self::ChunkPuller<'_> {
        Con2ChunkPuller {
            iter: self,
            chunk_size,
            thread_idx: None,
            chunk_buffer: Vec::new(),
        }
    }
}
