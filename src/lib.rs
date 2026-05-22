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
pub use cross_no_std::ConcurrentRecursiveIterCrossbeamNoStd;

#[cfg(feature = "std")]
pub use cross_std::ConcurrentRecursiveIterCrossbeamStd;
