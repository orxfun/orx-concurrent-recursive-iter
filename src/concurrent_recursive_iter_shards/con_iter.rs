use crate::concurrent_recursive_iter_shards::{
    backend::ShardedQueue, chunk_puller::DynChunkPuller, dyn_seq_queue::DynSeqQueue, queue::Queue,
};
use core::num::NonZeroUsize;
use core::sync::atomic::Ordering;
use orx_concurrent_iter::ConcurrentIter;
use orx_concurrent_queue::{ConcurrentQueue, DefaultConPinnedVec};
use orx_pinned_vec::{ConcurrentPinnedVec, IntoConcurrentPinnedVec, PinnedVec};
use orx_pseudo_default::PseudoDefault;

/// A recursive [`ConcurrentIter`] which:
/// * naturally shrinks as we iterate,
/// * but can also grow as it allows to add new items to the iterator, during iteration.
///
/// Growth of the iterator is expressed by the `extend: E` function with signature `E: Fn(&T, &Queue<T, P>)`.
///
/// [`Queue`] here is a wrapper around the the backing queue of elements which exposes only two methods:
/// [`push`] and [`extend`]. Having access to growth methods of the queue, we can add elements to the iterator
/// while we are processing.
///
/// Importantly note that extension happens before yielding the next element.
///
/// In other words, for each element `e` pulled from the iterator, we call `extend(&e, &queue)` before
/// returning it to the caller.
///
/// *The recursive concurrent iterator internally uses a [`ConcurrentQueue`] which allows for both
/// concurrent push / extend and pop / pull operations.*
///
/// [`push`]: Queue::push
/// [`extend`]: Queue::extend
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
/// use orx_concurrent_recursive_iter::*;
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
/// fn extend<'a, 'b>(node: &'a &'b Node, queue: &Queue<&'b Node>) {
///     queue.extend(&node.children);
/// }
///
/// // initiate iter with a single element, `root`
/// // however, the iterator will `extend` on the fly as we keep drawing its elements
/// let root = Node::new(&mut ChaCha8Rng::seed_from_u64(42), 70);
/// let iter = ConcurrentRecursiveIter::new([&root], extend);
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
pub struct ConcurrentRecursiveIter<T, E, P = DefaultConPinnedVec<T>>
where
    T: Send,
    P: ConcurrentPinnedVec<T>,
    <P as ConcurrentPinnedVec<T>>::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
    E: Fn(&T, &Queue<T, P>) + Sync,
{
    queue: ShardedQueue<T, P>,
    extend: E,
    exact_len: Option<usize>,
}

impl<T, E, P> From<(ConcurrentQueue<T, P>, E)> for ConcurrentRecursiveIter<T, E, P>
where
    T: Send,
    P: ConcurrentPinnedVec<T>,
    <P as ConcurrentPinnedVec<T>>::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
    E: Fn(&T, &Queue<T, P>) + Sync,
{
    fn from((queue, extend): (ConcurrentQueue<T, P>, E)) -> Self {
        Self {
            queue: ShardedQueue::from_single(queue),
            extend,
            exact_len: None,
        }
    }
}

impl<T, E, P> From<(ConcurrentQueue<T, P>, E, usize)> for ConcurrentRecursiveIter<T, E, P>
where
    T: Send,
    P: ConcurrentPinnedVec<T>,
    <P as ConcurrentPinnedVec<T>>::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
    E: Fn(&T, &Queue<T, P>) + Sync,
{
    fn from((queue, extend, exact_len): (ConcurrentQueue<T, P>, E, usize)) -> Self {
        Self {
            queue: ShardedQueue::from_single(queue),
            extend,
            exact_len: Some(exact_len),
        }
    }
}

impl<T, E> ConcurrentRecursiveIter<T, E, DefaultConPinnedVec<T>>
where
    T: Send,
    E: Fn(&T, &Queue<T, DefaultConPinnedVec<T>>) + Sync,
{
    /// Creates a new dynamic concurrent iterator:
    ///
    /// * The iterator will initially contain `initial_elements`.
    /// * Before yielding each element, say `e`, to the caller, the elements returned
    ///   by `extend(&e, &queue)` will called to create elements on the fly.
    ///
    /// This constructor uses a [`ConcurrentQueue`] with the default pinned concurrent
    /// collection under the hood. In order to create the iterator using a different queue
    /// use the `From`/`Into` traits, as demonstrated below.
    ///
    /// # UnknownSize vs ExactSize
    ///
    /// Size refers to the total number of elements that will be returned by the iterator,
    /// which is the total of initial elements and all elements created by the recursive
    /// extend calls.
    ///
    /// Note that the iterator created with this method will have an unknown size.
    /// In order to create a recursive iterator with a known exact length, you may use
    /// [`new_exact`] function.
    ///
    /// Providing an `exact_len` impacts the following:
    /// * When the exact length is provided, `try_get_len` method can provide the number of remaining
    ///   elements. When this is not necessary, the exact length argument can simply be skipped.
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
    /// use orx_concurrent_recursive_iter::{ConcurrentRecursiveIter, Queue};
    /// use orx_concurrent_iter::ConcurrentIter;
    ///
    /// let extend = |x: &usize, queue: &Queue<usize>| {
    ///     if *x < 5 {
    ///         queue.push(x + 1);
    ///     }
    /// };
    ///
    /// let initial_elements = [1];
    ///
    /// let iter = ConcurrentRecursiveIter::new(initial_elements, extend);
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
    /// use orx_pinned_vec::IntoConcurrentPinnedVec;
    /// use orx_split_vec::{SplitVec, Linear};
    /// use orx_fixed_vec::FixedVec;
    ///
    /// let initial_elements = [1];
    /// fn extend<P>(x: &usize, queue: &Queue<usize, P::ConPinnedVec>)
    /// where
    ///     P: IntoConcurrentPinnedVec<usize>,
    /// {
    ///     if *x < 5 {
    ///         queue.push(x + 1);
    ///     }
    /// }
    ///
    /// // SplitVec with Linear growth
    /// let queue = ConcurrentQueue::with_linear_growth(10, 4);
    /// queue.extend(initial_elements);
    /// let iter = ConcurrentRecursiveIter::from((queue, extend::<SplitVec<_, Linear>>));
    ///
    /// let all: Vec<_> = iter.item_puller().collect();
    /// assert_eq!(all, [1, 2, 3, 4, 5]);
    ///
    /// // FixedVec with fixed capacity
    /// let queue = ConcurrentQueue::with_fixed_capacity(5);
    /// queue.extend(initial_elements);
    /// let iter = ConcurrentRecursiveIter::from((queue, extend::<FixedVec<_>>));
    ///
    /// let all: Vec<_> = iter.item_puller().collect();
    /// assert_eq!(all, [1, 2, 3, 4, 5]);
    /// ```
    ///
    /// [`SplitVec`]: orx_split_vec::SplitVec
    /// [`FixedVec`]: orx_fixed_vec::FixedVec
    /// [`Doubling`]: orx_split_vec::Doubling
    /// [`Linear`]: orx_split_vec::Linear
    pub fn new(initial_elements: impl IntoIterator<Item = T>, extend: E) -> Self {
        Self::new_with_shards(initial_elements, extend, NonZeroUsize::MIN)
    }

    /// Creates a new recursive iterator with `num_shards` internal queues.
    ///
    /// Higher shard counts can reduce contention under high thread counts.
    pub fn new_with_shards(
        initial_elements: impl IntoIterator<Item = T>,
        extend: E,
        num_shards: NonZeroUsize,
    ) -> Self {
        let num_shards = num_shards.get();
        let mut shards = [(); 32].map(|_| ConcurrentQueue::pseudo_default());
        for i in 0..num_shards {
            shards[i] = ConcurrentQueue::new();
        }

        let queue = ShardedQueue::from_shards(num_shards, shards);
        for element in initial_elements {
            queue.push(element);
        }

        Self {
            queue,
            extend,
            exact_len: None,
        }
    }

    /// Creates a new dynamic concurrent iterator:
    ///
    /// * The iterator will initially contain `initial_elements`.
    /// * Before yielding each element, say `e`, to the caller, the elements returned
    ///   by `extend(&e, &queue)` will called to create elements on the fly.
    ///
    /// This constructor uses a [`ConcurrentQueue`] with the default pinned concurrent
    /// collection under the hood. In order to create the iterator using a different queue
    /// use the `From`/`Into` traits, as demonstrated below.
    ///
    /// # UnknownSize vs ExactSize
    ///
    /// Size refers to the total number of elements that will be returned by the iterator,
    /// which is the total of initial elements and all elements created by the recursive
    /// extend calls.
    ///
    /// Note that the iterator created with this method will have an unknown size.
    /// In order to create a recursive iterator with a known exact length, you may use
    /// [`new_exact`] function.
    ///
    /// Providing an `exact_len` impacts the following:
    /// * When the exact length is provided, `try_get_len` method can provide the number of remaining
    ///   elements. When this is not necessary, the exact length argument can simply be skipped.
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
    /// use orx_concurrent_recursive_iter::{ConcurrentRecursiveIter, Queue};
    /// use orx_concurrent_iter::ConcurrentIter;
    ///
    /// let extend = |x: &usize, queue: &Queue<usize>| {
    ///     if *x < 5 {
    ///         queue.push(x + 1);
    ///     }
    /// };
    ///
    /// let initial_elements = [1];
    ///
    /// let iter = ConcurrentRecursiveIter::new(initial_elements, extend);
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
    /// use orx_pinned_vec::IntoConcurrentPinnedVec;
    /// use orx_split_vec::{SplitVec, Linear};
    /// use orx_fixed_vec::FixedVec;
    ///
    /// let initial_elements = [1];
    /// fn extend<P>(x: &usize, queue: &Queue<usize, P::ConPinnedVec>)
    /// where
    ///     P: IntoConcurrentPinnedVec<usize>,
    /// {
    ///     if *x < 5 {
    ///         queue.push(x + 1);
    ///     }
    /// }
    ///
    /// // SplitVec with Linear growth
    /// let queue = ConcurrentQueue::with_linear_growth(10, 4);
    /// queue.extend(initial_elements);
    /// let iter = ConcurrentRecursiveIter::from((queue, extend::<SplitVec<_, Linear>>));
    ///
    /// let all: Vec<_> = iter.item_puller().collect();
    /// assert_eq!(all, [1, 2, 3, 4, 5]);
    ///
    /// // FixedVec with fixed capacity
    /// let queue = ConcurrentQueue::with_fixed_capacity(5);
    /// queue.extend(initial_elements);
    /// let iter = ConcurrentRecursiveIter::from((queue, extend::<FixedVec<_>>));
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
        initial_elements: impl IntoIterator<Item = T>,
        extend: E,
        exact_len: usize,
    ) -> Self {
        Self::new_exact_with_shards(initial_elements, extend, exact_len, NonZeroUsize::MIN)
    }

    /// Creates a new recursive iterator with `num_shards` internal queues
    /// and an exact total length.
    pub fn new_exact_with_shards(
        initial_elements: impl IntoIterator<Item = T>,
        extend: E,
        exact_len: usize,
        num_shards: NonZeroUsize,
    ) -> Self {
        let num_shards = num_shards.get();
        let mut shards = [(); 32].map(|_| ConcurrentQueue::pseudo_default());
        for i in 0..num_shards {
            shards[i] = ConcurrentQueue::new();
        }

        let queue = ShardedQueue::from_shards(num_shards, shards);
        for element in initial_elements {
            queue.push(element);
        }

        Self {
            queue,
            extend,
            exact_len: Some(exact_len),
        }
    }
}

impl<T, E, P> ConcurrentIter for ConcurrentRecursiveIter<T, E, P>
where
    T: Send,
    P: ConcurrentPinnedVec<T>,
    <P as ConcurrentPinnedVec<T>>::P: IntoConcurrentPinnedVec<T, ConPinnedVec = P>,
    E: Fn(&T, &Queue<T, P>) + Sync,
{
    type Item = T;

    type SequentialIter = DynSeqQueue<T, P, E>;

    type ChunkPuller<'i>
        = DynChunkPuller<'i, T, E, P>
    where
        Self: 'i;

    fn into_seq_iter(self) -> Self::SequentialIter {
        DynSeqQueue::new(self.queue, self.extend)
    }

    fn skip_to_end(&self) {
        self.queue.skip_to_end();
    }

    fn next(&self) -> Option<Self::Item> {
        let (shard_idx, n) = self.queue.pop()?;
        (self.extend)(&n, &Queue::with_shard(&self.queue, shard_idx));
        Some(n)
    }

    fn next_with_idx(&self) -> Option<(usize, Self::Item)> {
        let (idx, shard_idx, n) = self.queue.pop_with_idx()?;
        (self.extend)(&n, &Queue::with_shard(&self.queue, shard_idx));
        Some((idx, n))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self.exact_len {
            Some(exact_len) => {
                let popped = self.queue.num_popped(Ordering::Relaxed);
                let remaining = exact_len - popped;
                (remaining, Some(remaining))
            }
            None => match self.queue.len() {
                0 => (0, Some(0)),
                n => (n, None),
            },
        }
    }

    fn is_completed_when_none_returned(&self) -> bool {
        let popped = self.queue.num_popped(Ordering::Relaxed);
        let write_reserved = self.queue.num_write_reserved(Ordering::Relaxed);
        popped >= write_reserved
    }

    fn chunk_puller(&self, chunk_size: usize) -> Self::ChunkPuller<'_> {
        DynChunkPuller::new(&self.extend, &self.queue, chunk_size)
    }
}
