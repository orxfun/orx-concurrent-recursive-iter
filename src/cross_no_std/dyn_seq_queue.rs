use crate::cross_no_std::queue::Queue;
use alloc::sync::Arc;
use core::iter::FusedIterator;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub struct DynSeqCrossbeamNoStd<I, E>
where
    I: IntoIterator,
    I::IntoIter: ExactSizeIterator,
    I::Item: Send,
    E: Fn(&I::Item) -> I + Send + Sync,
{
    pub(super) queue: Arc<Queue<I::Item>>,
    pub(super) extend: Arc<E>,
    pub(super) pending: Arc<AtomicUsize>,
    pub(super) popped: Arc<AtomicUsize>,
    pub(super) stopped: Arc<AtomicBool>,
}

impl<I, E> Iterator for DynSeqCrossbeamNoStd<I, E>
where
    I: IntoIterator,
    I::IntoIter: ExactSizeIterator,
    I::Item: Send,
    E: Fn(&I::Item) -> I + Send + Sync,
{
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        super::con_iter::ConcurrentRecursiveIterCrossbeamNoStd::next_impl(
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
        match self.stopped.load(Ordering::Acquire) || self.pending.load(Ordering::Acquire) == 0 {
            true => (0, Some(0)),
            false => (0, None),
        }
    }
}

impl<I, E> FusedIterator for DynSeqCrossbeamNoStd<I, E>
where
    I: IntoIterator,
    I::IntoIter: ExactSizeIterator,
    I::Item: Send,
    E: Fn(&I::Item) -> I + Send + Sync,
{
}
