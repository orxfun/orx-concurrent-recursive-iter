use crate::cross_std::local_worker::LocalWorker;
use crate::cross_std::queue::Queue;
use core::iter::FusedIterator;
use crossbeam_deque::Stealer;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub struct DynSeqCrossbeam<I, E>
where
    I: IntoIterator,
    I::IntoIter: ExactSizeIterator,
    I::Item: Send,
    E: Fn(&I::Item) -> I + Send + Sync,
{
    pub(super) injector: Arc<Queue<I::Item>>,
    pub(super) extend: Arc<E>,
    pub(super) locals: Arc<Vec<LocalWorker<I::Item>>>,
    pub(super) stealers: Arc<Vec<Stealer<I::Item>>>,
    pub(super) pending: Arc<AtomicUsize>,
    pub(super) popped: Arc<AtomicUsize>,
    pub(super) stopped: Arc<AtomicBool>,
}

impl<I, E> Iterator for DynSeqCrossbeam<I, E>
where
    I: IntoIterator,
    I::IntoIter: ExactSizeIterator,
    I::Item: Send,
    E: Fn(&I::Item) -> I + Send + Sync,
{
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        super::con_iter::ConcurrentRecursiveIterCrossbeam::next_impl(
            &self.injector,
            &*self.extend,
            &self.locals,
            &self.stealers,
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

impl<I, E> FusedIterator for DynSeqCrossbeam<I, E>
where
    I: IntoIterator,
    I::IntoIter: ExactSizeIterator,
    I::Item: Send,
    E: Fn(&I::Item) -> I + Send + Sync,
{
}
