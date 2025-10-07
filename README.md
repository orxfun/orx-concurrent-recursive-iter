# orx-concurrent-recursive-iter

[![orx-concurrent-recursive-iter crate](https://img.shields.io/crates/v/orx-concurrent-recursive-iter.svg)](https://crates.io/crates/orx-concurrent-recursive-iter)
[![orx-concurrent-recursive-iter crate](https://img.shields.io/crates/d/orx-concurrent-recursive-iter.svg)](https://crates.io/crates/orx-concurrent-recursive-iter)
[![orx-concurrent-recursive-iter documentation](https://docs.rs/orx-concurrent-recursive-iter/badge.svg)](https://docs.rs/orx-concurrent-recursive-iter)

A concurrent iterator ([ConcurrentIter](https://docs.rs/orx-concurrent-iter/latest/orx_concurrent_iter/trait.ConcurrentIter.html)) that can be extended recursively by each of its items.

> This is a **no-std** crate.

## Concurrent Recursive Iter

[`ConcurrentRecursiveIter`](https://docs.rs/orx-concurrent-iter/latest/orx_concurrent_recursive_iter/struct.ConcurrentRecursiveIter.html) is a [ConcurrentIter](https://docs.rs/orx-concurrent-iter/latest/orx_concurrent_iter/trait.ConcurrentIter.html) implementation which

* naturally shrinks as we iterate,
* but can also grow as it allows to add new items to the iterator, during iteration.

Assume the item type of the iterator is `T`. Growth of the iterator is expressed by the `extend: E` function with the signature `Fn(&T) -> I` where `I: IntoIterator<Item = T>` with a known length.

In other words, for each element `e` pulled from the iterator, the iterator internally calls `extend(&e)` before returning it to the caller. All elements included in the iterator that `extend` returned are added to the end of the concurrent iterator, to be pulled later on.

> The recursive concurrent iterator internally uses a [`ConcurrentQueue`](https://docs.rs/orx-concurrent-queue/latest/orx_concurrent_queue/struct.ConcurrentQueue.html) which allows for both concurrent push / extend and pop / pull operations.

### A simple example, extending by 0 or 1 elements

Consider the following example. We initiate the iterator with two elements, 1 and 2.

Growth is defined by the `extend` function. For every element `x` pulled from the iterator, we will add `10 * x` to the iterator if `x` is less than a thousand.

```rust
use orx_concurrent_recursive_iter::*;

let initial = [1, 2];
let extend = |x: &usize| (*x < 1000).then_some(x * 10);

let iter = ConcurrentRecursiveIter::new(extend, initial);

let mut collected = vec![];
while let Some(x) = iter.next() {
    collected.push(x);
}

assert_eq!(collected, vec![1, 2, 10, 20, 100, 200, 1000, 2000]);
```

This sequential example allows us demonstrate the recursive iteration easily. Following is the list of events in order during the `while let` iteration:
* `iter` has elements `[1, 2]`
* we make the `next` call
  * 1 is pulled, `iter` has one element `[2]`
  * `extend(&1)` is called which returns 10, this is added to iter, `[2, 10]`.
  * only then, 1 is returned; i.e., `x` is set to 1 which is then used by the caller (added to the `collected` here).
* we make the second `next` call
  * 2 is pulled, `iter` has one element `[10]`
  * `extend(&2)` is called which returns 20, this is added to iter, `[10, 20]`.
  * then 2 is assigned to `x`.
* ...
* we make another `next` call while `iter` has one element `[2000]`.
  * 2000 is pulled, leaving `iter` empty
  * `extend(&2000)` is called which returns None.
  * then 2000 is assigned to `x`.
* finally, the iterator is empty and the `next` call returns `None`.

### A simple example, extending by 0 or multiple elements

The following is again a simple and sequential example, except that this time each element extends the recursive iterator by 0 or multiple elements.

```rust
use orx_concurrent_recursive_iter::*;

let initial = [1];
let extend = |x: &usize| match *x < 100 {
    true => vec![x * 10, x * 20],
    false => vec![],
};
let iter = ConcurrentRecursiveIter::new(extend, initial);

let collected: Vec<_> = iter.item_puller().collect();
assert_eq!(collected, vec![1, 10, 20, 100, 200, 200, 400]);
```

Here, we start with only one initial element, 1:

* we make the first `next` call:
  * 1 is pulled leaving the `iter` empty.
  * `extend(&1)` call returns two elements which are added to the iterator which then becomes `[10, 20]`.
* we make the second `next` call:
  * 10 is pulled and `iter` becomes `[20]`.
  * `extend(&10)` returns two more elements which results in `iter = [20, 100, 200]`.
* ...
* we make another `next` call while `iter` has one element `[400]`.
  * 400 is pulled, leaving `iter` empty
  * `extend(&400)` is called which returns nothing.
  * then 400 is assigned to `x`.
* finally, the iterator is empty and the `next` call returns `None`.

### The allocation problem

Notice in the above example that the `extend` method returns a vector. Therefore, each time it returns multiple elements, we need to allocate. This is due to the requirement that `extend` must return an `ExactSizeIterator`.

The same program can be implemented in the following way to avoid allocation:

```rust compile_fail
use orx_concurrent_recursive_iter::*;

let initial = [1];
let extend = |x: &usize| {
    (*x < 100)
        .then_some(*x)
        .into_iter()
        .flat_map(|x| [x * 10, x * 20])
};
let iter = ConcurrentRecursiveIter::new(extend, initial);

let collected: Vec<_> = iter.item_puller().collect();
assert_eq!(collected, vec![1, 10, 20, 100, 200, 200, 400]);
```

However, this code unfortunately does not compile:

```bash
FlatMap<std::option::IntoIter<usize>, [usize; 2], impl FnMut(usize) -> [usize; 2]>: ExactSizeIterator` is not satisfied
```

The problem is that `FlatMap<std::option::IntoIter<usize>, impl FnMut(usize) -> [usize; 2]>` does not implement `ExactSizeIterator`.

We might, however, argue that we know exactly the size of this iterator from the type signature:
* it is 0 if the option is `None`,
* it is 2 otherwise.

And we would be absolutely correct!

This seems like a missing implementation from the core library and will hopefully be added.

And in certain use cases when iterating over recursive data structures, we do not observe the allocation problem since the allocation is already done while defining the input collection. The following example demonstrates such a case.

### Concurrent recursive iteration

Consider a recursive tree structure defined by the `Node` struct. `Node::new` call creates some random tree with a total of 177 nodes. These nodes are stored as descendants of the `root`.

We create our recursive iterator with one initial element which is the root. We define the `extend` function from a node as its children. As mentioned above, notice that we only return the slice of children and do not need allocation here.

This allows us to `process` each of the 177 nodes concurrently.

```rust
use orx_concurrent_recursive_iter::*;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Node {
    value: u64,
    children: Vec<Node>,
}

impl Node {
    fn new(rng: &mut impl Rng, value: u64) -> Self {
        let num_children = match value {
            0 => 0,
            n => rng.random_range(0..(n as usize)),
        };
        let children = (0..num_children)
            .map(|i| Self::new(rng, i as u64))
            .collect();
        Self { value, children }
    }
}

fn process(node_value: u64) {
    // fake computation
    std::thread::sleep(std::time::Duration::from_millis(node_value));
}

// this defines how the iterator must extend:
// each node drawn from the iterator adds its children to the end of the iterator
fn extend<'a, 'b>(node: &'a &'b Node) -> &'b [Node] {
    &node.children
}

// initiate iter with a single element, `root`
// however, the iterator will `extend` on the fly as we keep drawing its elements
let root = Node::new(&mut ChaCha8Rng::seed_from_u64(42), 70);
let iter = ConcurrentRecursiveIter::new(extend, [&root]);

let num_threads = 8;
let num_spawned = AtomicUsize::new(0);
let num_processed_nodes = AtomicUsize::new(0);

std::thread::scope(|s| {
    let mut handles = vec![];
    for _ in 0..num_threads {
        handles.push(s.spawn(|| {
            // allow all threads to be spawned
            _ = num_spawned.fetch_add(1, Ordering::Relaxed);
            while num_spawned.load(Ordering::Relaxed) < num_threads {}

            // `next` will first extend `iter` with children of `node,
            // and only then yield the `node`
            while let Some(node) = iter.next() {
                process(node.value);
                _ = num_processed_nodes.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }
});

assert_eq!(num_processed_nodes.into_inner(), 177);
```

## Contributing

Contributions are welcome! If you notice an error, have a question or think something could be improved, please open an [issue](https://github.com/orxfun/orx-concurrent-recursive-iter/issues/new) or create a PR.

## License

Dual-licensed under [Apache 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT).
