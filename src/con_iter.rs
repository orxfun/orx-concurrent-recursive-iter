#[cfg(not(feature = "std"))]
pub type ConcurrentRecursiveIter<I, E> =
    crate::cross_no_std::ConcurrentRecursiveIterCrossbeamNoStd<I, E>;

#[cfg(feature = "std")]
pub type ConcurrentRecursiveIter<I, E> =
    crate::cross_std::ConcurrentRecursiveIterCrossbeamStd<I, E>;
