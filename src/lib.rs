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

extern crate alloc;
// #[cfg(test)]
// extern crate std;

mod cross_no_std;
mod cross_std;
mod orx_queue;

// re-import
pub use cross_no_std::ConcurrentRecursiveIterCrossbeamNoStd;
pub use cross_std::ConcurrentRecursiveIterCrossbeam;
pub use orx_concurrent_iter::*;
