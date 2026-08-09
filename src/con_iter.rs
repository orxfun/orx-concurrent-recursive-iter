/// A recursive [`ConcurrentIter`](orx_concurrent_iter::ConcurrentIter) alias that chooses
/// the backend by crate feature flags.
///
/// The iterator starts with initial elements and can grow recursively while iterating:
/// for each yielded element `e`, `extend(&e)` is called and produced children are pushed
/// back into the shared work queue before the iteration proceeds.
///
/// # Example
///
/// ```
/// use orx_concurrent_iter::ConcurrentIter;
/// use orx_concurrent_recursive_iter::ConcurrentRecursiveIter;
///
/// let extend = |x: &usize| (*x < 5).then_some(x + 1);
/// let iter = ConcurrentRecursiveIter::new([1], extend, None, None);
///
/// let all: Vec<_> = iter.item_puller().collect();
/// assert_eq!(all, [1, 2, 3, 4, 5]);
/// ```
#[cfg(not(feature = "std"))]
pub type ConcurrentRecursiveIter<I, E> =
    crate::cross_no_std::ConcurrentRecursiveIterCrossbeamNoStd<I, E>;

/// A recursive [`ConcurrentIter`](orx_concurrent_iter::ConcurrentIter) alias that chooses
/// the backend by crate feature flags.
///
/// The iterator starts with initial elements and can grow recursively while iterating:
/// for each yielded element `e`, `extend(&e)` is called and produced children are pushed
/// back into the shared work queue before the iteration proceeds.
///
/// # Example
///
/// ```
/// use orx_concurrent_iter::ConcurrentIter;
/// use orx_concurrent_recursive_iter::ConcurrentRecursiveIter;
///
/// let extend = |x: &usize| (*x < 5).then_some(x + 1);
/// let iter = ConcurrentRecursiveIter::new([1], extend, None, None);
///
/// let all: Vec<_> = iter.item_puller().collect();
/// assert_eq!(all, [1, 2, 3, 4, 5]);
/// ```
#[cfg(feature = "std")]
pub type ConcurrentRecursiveIter<I, E> =
    crate::cross_std::ConcurrentRecursiveIterCrossbeamStd<I, E>;
