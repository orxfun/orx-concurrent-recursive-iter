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

#[cfg(feature = "experimental")]
mod orx_queue;

#[cfg(not(feature = "std"))]
pub type ConcurrentRecursiveIter<I, E> = cross_no_std::ConcurrentRecursiveIterCrossbeamNoStd<I, E>;

#[cfg(feature = "std")]
pub type ConcurrentRecursiveIter<I, E> = cross_std::ConcurrentRecursiveIterCrossbeamStd<I, E>;

pub use orx_concurrent_iter::*;

#[cfg(feature = "experimental")]
pub use orx_queue::{ConcurrentRecursiveIterQueue, Queue};

#[cfg(test)]
mod abc {
    use crate::*;
    use alloc::vec;
    use alloc::vec::Vec;

    #[test]
    fn def() {
        use rand::{Rng, SeedableRng};
        use rand_chacha::ChaCha8Rng;
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct Node {
            value: u64,
            children: Vec<Node>,
        }

        impl Node {
            fn new(rng: &mut impl Rng, value: u64) -> Self {
                let num_children = match value {
                    0 => 0,
                    n => rng.random_range(0..(n as usize)),
                };
                let children = (0..num_children)
                    .map(|i| Self::new(rng, i as u64))
                    .collect();
                Self { value, children }
            }
        }

        fn process(node_value: u64) {
            // fake computation
            std::thread::sleep(std::time::Duration::from_millis(node_value));
        }

        // initiate iter with a single element, `root`
        // however, the iterator will `extend` on the fly as we keep drawing its elements
        let root = Node::new(&mut ChaCha8Rng::seed_from_u64(42), 70);
        let iter =
            ConcurrentRecursiveIter::new([&root], |node: &&Node| node.children.iter(), None, None);

        let num_threads = 8;
        let num_spawned = AtomicUsize::new(0);
        let num_processed_nodes = AtomicUsize::new(0);

        std::thread::scope(|s| {
            let mut handles = vec![];
            for _ in 0..num_threads {
                handles.push(s.spawn(|| {
                    // allow all threads to be spawned
                    _ = num_spawned.fetch_add(1, Ordering::Relaxed);
                    while num_spawned.load(Ordering::Relaxed) < num_threads {}

                    // `next` will first extend `iter` with children of `node,
                    // and only then yield the `node`
                    while let Some(node) = iter.next() {
                        process(node.value);
                        _ = num_processed_nodes.fetch_add(1, Ordering::Relaxed);
                    }
                }));
            }
        });

        assert_eq!(num_processed_nodes.into_inner(), 177);
    }
}
