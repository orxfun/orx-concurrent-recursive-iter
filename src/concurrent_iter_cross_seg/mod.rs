//! Concurrent recursive iterator using crossbeam's SegQueue.

pub mod con_iter;

pub use con_iter::ConcurrentIterCrossSeg;