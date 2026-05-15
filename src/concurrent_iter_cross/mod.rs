//! Concurrent recursive iterator using crossbeam's work-stealing deque.

pub mod con_iter;

pub use con_iter::ConcurrentIterCross;
