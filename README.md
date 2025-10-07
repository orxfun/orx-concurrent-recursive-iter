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

```rust

```




## Contributing

Contributions are welcome! If you notice an error, have a question or think something could be improved, please open an [issue](https://github.com/orxfun/orx-concurrent-recursive-iter/issues/new) or create a PR.

## License

Dual-licensed under [Apache 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT).
