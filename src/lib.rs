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
// #![no_std]

#[cfg(test)]
extern crate alloc;
// #[cfg(test)]
// extern crate std;

// #[cfg(test)]
// mod tests;

// mod chunk;
// mod chunk_puller;
// mod con_iter;
mod archive2;
mod con1;
mod concurrent_iter_cross;
mod concurrent_iter_cross_seg;
mod concurrent_recursive_iter_shards;
mod concurrent_recursive_iter_shards2;
// mod dyn_seq_queue;
// mod queue;

// re-import
pub use orx_concurrent_iter::*;

pub use archive2::{ConcurrentRecursiveIter, Queue};
pub use con1::Con1;
pub use concurrent_iter_cross::ConcurrentIterCross;
pub use concurrent_iter_cross_seg::ConcurrentIterCrossSeg;
pub use concurrent_recursive_iter_shards::ConcurrentRecursiveIterShards;
pub use concurrent_recursive_iter_shards2::ConcurrentRecursiveIterShards2;
