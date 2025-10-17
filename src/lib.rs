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

#[cfg(test)]
extern crate alloc;
#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests;

mod chunk;
mod chunk_puller;
mod con_iter;
mod dyn_seq_queue;
mod sized;

pub use con_iter::ConcurrentRecursiveIter;

// re-import
pub use orx_concurrent_iter::*;
