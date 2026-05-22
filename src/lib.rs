#![doc = include_str!("../README.md")]
#![warn(
    missing_docs,
    clippy::unwrap_in_result,
    clippy::unwrap_used,
    clippy::panic,
    clippy::panic_in_result_fn,
    clippy::float_cmp,
    clippy::float_cmp_const,
    clippy::missing_panics_doc,
    clippy::todo
)]
#![no_std]

extern crate alloc;

#[cfg(any(test, feature = "std"))]
extern crate std;

#[cfg(not(feature = "std"))]
mod cross_no_std;
#[cfg(feature = "std")]
mod cross_std;

// mod orx_queue;

#[cfg(not(feature = "std"))]
pub type ConcurrentRecursiveIter<I, E> = cross_no_std::ConcurrentRecursiveIterCrossbeamNoStd<I, E>;

#[cfg(feature = "std")]
pub type ConcurrentRecursiveIter<I, E> = cross_std::ConcurrentRecursiveIterCrossbeamStd<I, E>;

pub use orx_concurrent_iter::*;

#[cfg(test)]
mod abc {
    use crate::*;
    use alloc::vec;
    use alloc::vec::Vec;

    #[test]
    fn def() {
        let initial = [1, 2];
        let extend = |x: &usize| (*x < 1000).then_some(x * 10);
        let extend = |x: &usize| {
            let x = (*x < 100).then_some([x * 10, x * 20]);
            let y = x.into_iter().map(|x| x.into_iter()).flatten();
            y
        };

        let iter = ConcurrentRecursiveIter::new(initial, extend, None, None);

        let mut collected = vec![];
        while let Some(x) = iter.next() {
            collected.push(x);
        }

        assert_eq!(collected, vec![1, 2, 10, 20, 100, 200, 1000, 2000]);
    }
}
