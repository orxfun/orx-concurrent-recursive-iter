use crate::orx_queue::queue::Queue;
use core::iter::FusedIterator;
use orx_concurrent_queue::ConcurrentQueue;
use orx_pinned_vec::{ConcurrentPinnedVec, IntoConcurrentPinnedVec};

pub struct DynSeqQueue<T, P, E>
where
    T: Send,
    P: ConcurrentPinnedVec<T>,
    <P as ConcurrentPinnedVec<T>>::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
    E: Fn(&T, &Queue<T, P>) + Sync,
{
    queue: ConcurrentQueue<T, P>,
    extend: E,
}

impl<T, P, E> DynSeqQueue<T, P, E>
where
    T: Send,
    P: ConcurrentPinnedVec<T>,
    <P as ConcurrentPinnedVec<T>>::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
    E: Fn(&T, &Queue<T, P>) + Sync,
{
    pub(super) fn new(queue: ConcurrentQueue<T, P>, extend: E) -> Self {
        Self { queue, extend }
    }
}

impl<T, P, E> Iterator for DynSeqQueue<T, P, E>
where
    T: Send,
    P: ConcurrentPinnedVec<T>,
    <P as ConcurrentPinnedVec<T>>::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
    E: Fn(&T, &Queue<T, P>) + Sync,
{
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        self.queue
            .pop()
            .inspect(|element| (self.extend)(element, &Queue::from(&self.queue)))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self.queue.len() {
            0 => (0, Some(0)),
            n => (n, None),
        }
    }
}

impl<T, P, E> FusedIterator for DynSeqQueue<T, P, E>
where
    T: Send,
    P: ConcurrentPinnedVec<T>,
    <P as ConcurrentPinnedVec<T>>::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
    E: Fn(&T, &Queue<T, P>) + Sync,
{
}
