//! Concurrent recursive iterator built on crossbeam SegQueue without std imports.

pub mod con_iter;

pub use con_iter::Con2;
