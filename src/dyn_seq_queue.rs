use crate::queue::Queue;
use core::{iter::FusedIterator, marker::PhantomData};
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
    written: usize,
    popped: usize,
    extend: E,
    phantom: PhantomData<T>,
}

impl<T, P, E> DynSeqQueue<T, P, E>
where
    T: Send,
    P: ConcurrentPinnedVec<T>,
    <P as ConcurrentPinnedVec<T>>::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
    E: Fn(&T, &Queue<T, P>) + Sync,
{
    pub(super) fn new(
        queue: ConcurrentQueue<T, P>,
        written: usize,
        popped: usize,
        extend: E,
    ) -> Self {
        Self {
            queue,
            written,
            popped,
            extend,
            phantom: PhantomData,
        }
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
        return self.queue.pop().map(|element| {
            (self.extend)(&element, &Queue::from(&self.queue));
            element
        });
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let min = self.written - self.popped;
        (min, None)
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
