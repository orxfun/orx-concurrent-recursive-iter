use crate::{
    ExactSize, Size, UnknownSize, chunk_puller::DynChunkPuller, dyn_seq_queue::DynSeqQueue,
};
use core::{marker::PhantomData, sync::atomic::Ordering};
use orx_concurrent_iter::{ConcurrentIter, ExactSizeConcurrentIter};
use orx_concurrent_queue::{ConcurrentQueue, DefaultConPinnedVec};
use orx_pinned_vec::{ConcurrentPinnedVec, IntoConcurrentPinnedVec};
use orx_split_vec::SplitVec;

/// A recursive [`ConcurrentIter`] which:
/// * naturally shrinks as we iterate,
/// * but can also grow as it allows to add new items to the iterator, during iteration.
///
/// Growth of the iterator is expressed by the `extend: E` function with the signature `Fn(&T) -> I`,
/// where `I: IntoIterator<Item = T>` with a known length.
///
/// In other words, for each element `e` pulled from the iterator, we call `extend(&e)` before
/// returning it to the caller. All elements included in the iterator that `extend` returned
/// are added to the end of the concurrent iterator, to be pulled later on.
///
/// *The recursive concurrent iterator internally uses a [`ConcurrentQueue`] which allows for both
/// concurrent push / extend and pop / pull operations.*
///
/// # Example
///
/// The following example demonstrates a use case for the recursive concurrent iterator.
/// Notice that the iterator is instantiated with:
/// * a single element which is the root node,
/// * and the extend method which defines how to extend the iterator from each node.
///
/// Including the root, there exist 177 nodes in the tree. We observe that all these
/// nodes are concurrently added to the iterator, popped and processed.
///
/// ```
/// use orx_concurrent_recursive_iter::ConcurrentRecursiveIter;
/// use orx_concurrent_iter::ConcurrentIter;
/// use std::sync::atomic::{AtomicUsize, Ordering};
/// use rand::{Rng, SeedableRng};
/// use rand_chacha::ChaCha8Rng;
///
/// struct Node {
///     value: u64,
///     children: Vec<Node>,
/// }
///
/// impl Node {
///     fn new(rng: &mut impl Rng, value: u64) -> Self {
///         let num_children = match value {
///             0 => 0,
///             n => rng.random_range(0..(n as usize)),
///         };
///         let children = (0..num_children)
///             .map(|i| Self::new(rng, i as u64))
///             .collect();
///         Self { value, children }
///     }
/// }
///
/// fn process(node_value: u64) {
///     // fake computation
///     std::thread::sleep(std::time::Duration::from_millis(node_value));
/// }
///
/// // this defines how the iterator must extend:
/// // each node drawn from the iterator adds its children to the end of the iterator
/// fn extend<'a, 'b>(node: &'a &'b Node) -> &'b [Node] {
///     &node.children
/// }
///
/// // initiate iter with a single element, `root`
/// // however, the iterator will `extend` on the fly as we keep drawing its elements
/// let root = Node::new(&mut ChaCha8Rng::seed_from_u64(42), 70);
/// let iter = ConcurrentRecursiveIter::new(extend, [&root]);
///
/// let num_threads = 8;
/// let num_spawned = AtomicUsize::new(0);
/// let num_processed_nodes = AtomicUsize::new(0);
///
/// std::thread::scope(|s| {
///     let mut handles = vec![];
///     for _ in 0..num_threads {
///         handles.push(s.spawn(|| {
///             // allow all threads to be spawned
///             _ = num_spawned.fetch_add(1, Ordering::Relaxed);
///             while num_spawned.load(Ordering::Relaxed) < num_threads {}
///
///             // `next` will first extend `iter` with children of `node,
///             // and only then yield the `node`
///             while let Some(node) = iter.next() {
///                 process(node.value);
///                 _ = num_processed_nodes.fetch_add(1, Ordering::Relaxed);
///             }
///         }));
///     }
/// });
///
/// assert_eq!(num_processed_nodes.into_inner(), 177);
/// ```
pub struct ConcurrentRecursiveIter<S, T, E, I, P = DefaultConPinnedVec<T>>
where
    S: Size,
    T: Send,
    E: Fn(&T) -> I + Sync,
    I: IntoIterator<Item = T>,
    I::IntoIter: ExactSizeIterator,
    P: ConcurrentPinnedVec<T>,
    <P as ConcurrentPinnedVec<T>>::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
{
    queue: ConcurrentQueue<T, P>,
    extend: E,
    exact_len: Option<usize>,
    p: PhantomData<S>,
}

// new with unknown size

impl<T, E, I, P> From<(E, ConcurrentQueue<T, P>)>
    for ConcurrentRecursiveIter<UnknownSize, T, E, I, P>
where
    T: Send,
    E: Fn(&T) -> I + Sync,
    I: IntoIterator<Item = T>,
    I::IntoIter: ExactSizeIterator,
    P: ConcurrentPinnedVec<T>,
    <P as ConcurrentPinnedVec<T>>::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
{
    fn from((extend, queue): (E, ConcurrentQueue<T, P>)) -> Self {
        Self {
            queue,
            extend,
            exact_len: None,
            p: PhantomData,
        }
    }
}

impl<T, E, I> ConcurrentRecursiveIter<UnknownSize, T, E, I, DefaultConPinnedVec<T>>
where
    T: Send,
    E: Fn(&T) -> I + Sync,
    I: IntoIterator<Item = T>,
    I::IntoIter: ExactSizeIterator,
{
    /// Creates a new dynamic concurrent iterator:
    ///
    /// * The iterator will initially contain `initial_elements`.
    /// * Before yielding each element, say `e`, to the caller, the elements returned
    ///   by `extend(&e)` will be added to the concurrent iterator, to be yield later.
    ///
    /// This constructor uses a [`ConcurrentQueue`] with the default pinned concurrent
    /// collection under the hood. In order to crate the iterator using a different queue
    /// use the `From`/`Into` traits, as demonstrated below.
    ///
    /// # UnknownSize vs ExactSize
    ///
    /// Note that the iterator created with this method will have [`UnknownSize`].
    /// In order to create a recursive iterator with a known exact length, you may use
    /// [`new_exact`] function.
    ///
    /// Providing an `exact_len` impacts the following:
    /// * When an exact length is provided, the recursive iterator implements
    ///   [`ExactSizeConcurrentIter`] in addition to [`ConcurrentIter`].
    ///   This enables the `len` method to access the number of remaining elements in a
    ///   concurrent program. When this is not necessary, the exact length argument
    ///   can simply be skipped.
    /// * On the other hand, a known length is very useful for performance optimization
    ///   when the recursive iterator is used as the input of a parallel iterator of the
    ///   [orx_parallel](https://crates.io/crates/orx-parallel) crate.
    ///
    /// [`new_exact`]: ConcurrentRecursiveIter::new_exact
    ///
    /// # Examples
    ///
    /// The following is a simple example to demonstrate how the dynamic iterator works.
    ///
    /// ```
    /// use orx_concurrent_recursive_iter::ConcurrentRecursiveIter;
    /// use orx_concurrent_iter::ConcurrentIter;
    ///
    /// let extend = |x: &usize| (*x < 5).then_some(x + 1);
    /// let initial_elements = [1];
    ///
    /// let iter = ConcurrentRecursiveIter::new(extend, initial_elements);
    /// let all: Vec<_> = iter.item_puller().collect();
    ///
    /// assert_eq!(all, [1, 2, 3, 4, 5]);
    /// ```
    ///
    /// # Examples - From
    ///
    /// In the above example, the underlying pinned vector of the dynamic iterator created
    /// with `new` is a [`SplitVec`] with a [`Doubling`] growth strategy.
    ///
    /// Alternatively, we can use a `SplitVec` with a [`Linear`] growth strategy, or a
    /// pre-allocated [`FixedVec`] as the underlying storage. In order to do so, we can
    /// use the `From` trait.
    ///
    /// ```
    /// use orx_concurrent_recursive_iter::*;
    /// use orx_concurrent_queue::ConcurrentQueue;
    ///
    /// let initial_elements = [1];
    ///
    /// // SplitVec with Linear growth
    /// let queue = ConcurrentQueue::with_linear_growth(10, 4);
    /// queue.extend(initial_elements);
    /// let extend = |x: &usize| (*x < 5).then_some(x + 1);
    /// let iter = ConcurrentRecursiveIter::from((extend, queue));
    ///
    /// let all: Vec<_> = iter.item_puller().collect();
    /// assert_eq!(all, [1, 2, 3, 4, 5]);
    ///
    /// // FixedVec with fixed capacity
    /// let queue = ConcurrentQueue::with_fixed_capacity(5);
    /// queue.extend(initial_elements);
    /// let extend = |x: &usize| (*x < 5).then_some(x + 1);
    /// let iter = ConcurrentRecursiveIter::from((extend, queue));
    ///
    /// let all: Vec<_> = iter.item_puller().collect();
    /// assert_eq!(all, [1, 2, 3, 4, 5]);
    /// ```
    ///
    /// [`SplitVec`]: orx_split_vec::SplitVec
    /// [`FixedVec`]: orx_fixed_vec::FixedVec
    /// [`Doubling`]: orx_split_vec::Doubling
    /// [`Linear`]: orx_split_vec::Linear
    pub fn new(extend: E, initial_elements: impl IntoIterator<Item = T>) -> Self {
        let mut vec = SplitVec::with_doubling_growth_and_max_concurrent_capacity();
        vec.extend(initial_elements);
        let queue = vec.into();
        (extend, queue).into()
    }
}

// new with exact size

impl<T, E, I, P> From<(E, ConcurrentQueue<T, P>, usize)>
    for ConcurrentRecursiveIter<ExactSize, T, E, I, P>
where
    T: Send,
    E: Fn(&T) -> I + Sync,
    I: IntoIterator<Item = T>,
    I::IntoIter: ExactSizeIterator,
    P: ConcurrentPinnedVec<T>,
    <P as ConcurrentPinnedVec<T>>::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
{
    fn from((extend, queue, exact_len): (E, ConcurrentQueue<T, P>, usize)) -> Self {
        Self {
            queue,
            extend,
            exact_len: Some(exact_len),
            p: PhantomData,
        }
    }
}

impl<T, E, I> ConcurrentRecursiveIter<ExactSize, T, E, I, DefaultConPinnedVec<T>>
where
    T: Send,
    E: Fn(&T) -> I + Sync,
    I: IntoIterator<Item = T>,
    I::IntoIter: ExactSizeIterator,
{
    /// Creates a new dynamic concurrent iterator:
    ///
    /// * The iterator will initially contain `initial_elements`.
    /// * Before yielding each element, say `e`, to the caller, the elements returned
    ///   by `extend(&e)` will be added to the concurrent iterator, to be yield later.
    ///
    /// This constructor uses a [`ConcurrentQueue`] with the default pinned concurrent
    /// collection under the hood. In order to crate the iterator using a different queue
    /// use the `From`/`Into` traits, as demonstrated below.
    ///
    /// # UnknownSize vs ExactSize
    ///
    /// Note that the iterator created with this method will have [`ExactSize`].
    /// In order to create a recursive iterator with an unknown length, you may use
    /// [`new`] function.
    ///
    /// Providing an `exact_len` impacts the following:
    /// * When an exact length is provided, the recursive iterator implements
    ///   [`ExactSizeConcurrentIter`] in addition to [`ConcurrentIter`].
    ///   This enables the `len` method to access the number of remaining elements in a
    ///   concurrent program. When this is not necessary, the exact length argument
    ///   can simply be skipped.
    /// * On the other hand, a known length is very useful for performance optimization
    ///   when the recursive iterator is used as the input of a parallel iterator of the
    ///   [orx_parallel](https://crates.io/crates/orx-parallel) crate.
    ///
    /// [`new`]: ConcurrentRecursiveIter::new
    ///
    /// # Examples
    ///
    /// The following is a simple example to demonstrate how the dynamic iterator works.
    ///
    /// ```
    /// use orx_concurrent_recursive_iter::*;
    ///
    /// let extend = |x: &usize| (*x < 5).then_some(x + 1);
    /// let initial_elements = [1];
    ///
    /// let iter = ConcurrentRecursiveIter::new_exact(extend, initial_elements, 5);
    /// let all: Vec<_> = iter.item_puller().collect();
    ///
    /// assert_eq!(all, [1, 2, 3, 4, 5]);
    /// ```
    ///
    /// # Examples - From
    ///
    /// In the above example, the underlying pinned vector of the dynamic iterator created
    /// with `new` is a [`SplitVec`] with a [`Doubling`] growth strategy.
    ///
    /// Alternatively, we can use a `SplitVec` with a [`Linear`] growth strategy, or a
    /// pre-allocated [`FixedVec`] as the underlying storage. In order to do so, we can
    /// use the `From` trait.
    ///
    /// ```
    /// use orx_concurrent_recursive_iter::*;
    /// use orx_concurrent_queue::ConcurrentQueue;
    ///
    /// let initial_elements = [1];
    ///
    /// // SplitVec with Linear growth
    /// let queue = ConcurrentQueue::with_linear_growth(10, 4);
    /// queue.extend(initial_elements);
    /// let extend = |x: &usize| (*x < 5).then_some(x + 1);
    /// let iter = ConcurrentRecursiveIter::from((extend, queue));
    ///
    /// let all: Vec<_> = iter.item_puller().collect();
    /// assert_eq!(all, [1, 2, 3, 4, 5]);
    ///
    /// // FixedVec with fixed capacity
    /// let queue = ConcurrentQueue::with_fixed_capacity(5);
    /// queue.extend(initial_elements);
    /// let extend = |x: &usize| (*x < 5).then_some(x + 1);
    /// let iter = ConcurrentRecursiveIter::from((extend, queue));
    ///
    /// let all: Vec<_> = iter.item_puller().collect();
    /// assert_eq!(all, [1, 2, 3, 4, 5]);
    /// ```
    ///
    /// [`SplitVec`]: orx_split_vec::SplitVec
    /// [`FixedVec`]: orx_fixed_vec::FixedVec
    /// [`Doubling`]: orx_split_vec::Doubling
    /// [`Linear`]: orx_split_vec::Linear
    pub fn new_exact(
        extend: E,
        initial_elements: impl IntoIterator<Item = T>,
        exact_len: usize,
    ) -> Self {
        let mut vec = SplitVec::with_doubling_growth_and_max_concurrent_capacity();
        vec.extend(initial_elements);
        let queue = vec.into();
        (extend, queue, exact_len).into()
    }
}

// con iter

impl<S, T, E, I, P> ConcurrentIter for ConcurrentRecursiveIter<S, T, E, I, P>
where
    S: Size,
    T: Send,
    E: Fn(&T) -> I + Sync,
    I: IntoIterator<Item = T>,
    I::IntoIter: ExactSizeIterator,
    P: ConcurrentPinnedVec<T>,
    <P as ConcurrentPinnedVec<T>>::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
{
    type Item = T;

    type SequentialIter = DynSeqQueue<T, P, E, I>;

    type ChunkPuller<'i>
        = DynChunkPuller<'i, T, E, I, P>
    where
        Self: 'i;

    fn into_seq_iter(self) -> Self::SequentialIter {
        // SAFETY: we destruct the queue and immediately convert it into a sequential
        // queue together with `popped..written` valid range information.
        let (vec, written, popped) = unsafe { self.queue.destruct() };
        DynSeqQueue::new(vec, written, popped, self.extend)
    }

    fn skip_to_end(&self) {
        let len = self.queue.num_write_reserved(Ordering::Acquire);
        let _remaining_to_drop = self.queue.pull(len);
    }

    fn next(&self) -> Option<Self::Item> {
        let n = self.queue.pop()?;
        let children = (self.extend)(&n);
        self.queue.extend(children);
        Some(n)
    }

    fn next_with_idx(&self) -> Option<(usize, Self::Item)> {
        let (idx, n) = self.queue.pop_with_idx()?;
        let children = (self.extend)(&n);
        self.queue.extend(children);
        Some((idx, n))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let min = self.queue.len();
        match min {
            0 => (0, Some(0)),
            n => (n, None),
        }
    }

    fn chunk_puller(&self, chunk_size: usize) -> Self::ChunkPuller<'_> {
        DynChunkPuller::new(&self.extend, &self.queue, chunk_size)
    }
}

#[test]
fn abc() {
    use alloc::vec::Vec;

    let initial_elements = [1];

    // SplitVec with Linear growth
    let queue = ConcurrentQueue::with_linear_growth(10, 4);
    queue.extend(initial_elements);
    let extend = |x: &usize| (*x < 5).then_some(x + 1);
    let iter = ConcurrentRecursiveIter::from((extend, queue));

    let all: Vec<_> = iter.item_puller().collect();
    assert_eq!(all, [1, 2, 3, 4, 5]);

    // FixedVec with fixed capacity
    let queue = ConcurrentQueue::with_fixed_capacity(5);
    queue.extend(initial_elements);
    let extend = |x: &usize| (*x < 5).then_some(x + 1);
    let iter = ConcurrentRecursiveIter::from((extend, queue));

    let all: Vec<_> = iter.item_puller().collect();
    assert_eq!(all, [1, 2, 3, 4, 5]);
}
