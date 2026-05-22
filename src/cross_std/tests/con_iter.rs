use crate::cross_std::{
    con_iter::ConcurrentRecursiveIterCrossbeam,
    tests::node::{Node, Roots},
};
use alloc::{
    string::{String, ToString},
    vec::Vec,
};
use core::sync::atomic::{AtomicUsize, Ordering};
use orx_concurrent_bag::ConcurrentBag;
use orx_concurrent_iter::{ChunkPuller, ConcurrentIter};
use test_case::test_matrix;

#[cfg(miri)]
const N: usize = 17;
#[cfg(not(miri))]
const N: usize = 125;

#[cfg(miri)]
const N_NODE: usize = 17;
#[cfg(not(miri))]
const N_NODE: usize = 125;

#[cfg(miri)]
const N_ROOT: usize = 2;
#[cfg(not(miri))]
const N_ROOT: usize = 8;

fn vec(n: usize) -> Vec<String> {
    (0..n).map(|i| (i + 1).to_string()).collect()
}

#[test]
fn basic_iter() {
    // 1 2 3 0 0 1 0 1 2 0 0 0 1 0
    let extend = |s: &String| {
        let i: usize = s.parse().unwrap();
        (0..i).map(|x| x.to_string())
    };
    let iter = ConcurrentRecursiveIterCrossbeam::new(vec(3), extend);

    assert_eq!(iter.next(), Some(1.to_string()));
    assert_eq!(iter.next(), Some(2.to_string()));
    assert_eq!(iter.next(), Some(3.to_string()));
    assert_eq!(iter.next(), Some(0.to_string()));
    assert_eq!(iter.next(), Some(0.to_string()));
    assert_eq!(iter.next(), Some(1.to_string()));
    assert_eq!(iter.next(), Some(0.to_string()));
    assert_eq!(iter.next(), Some(1.to_string()));
    assert_eq!(iter.next(), Some(2.to_string()));
    assert_eq!(iter.next(), Some(0.to_string()));
    assert_eq!(iter.next(), Some(0.to_string()));
    assert_eq!(iter.next(), Some(0.to_string()));
    assert_eq!(iter.next(), Some(1.to_string()));
    assert_eq!(iter.next(), Some(0.to_string()));
    assert_eq!(iter.next(), None);
    assert_eq!(iter.next(), None);
    assert_eq!(iter.next(), None);
    assert_eq!(iter.next(), None);
}

#[test]
fn basic_iter_with_idx() {
    // 1 2 3 0 0 1 0 1 2 0 0 0 1 0
    let extend = |s: &String| {
        let i: usize = s.parse().unwrap();
        (0..i).map(|x| x.to_string())
    };
    let iter = ConcurrentRecursiveIterCrossbeam::new(vec(3), extend);

    assert_eq!(iter.next_with_idx(), Some((0, 1.to_string())));
    assert_eq!(iter.next_with_idx(), Some((1, 2.to_string())));
    assert_eq!(iter.next_with_idx(), Some((2, 3.to_string())));
    assert_eq!(iter.next_with_idx(), Some((3, 0.to_string())));
    assert_eq!(iter.next_with_idx(), Some((4, 0.to_string())));
    assert_eq!(iter.next_with_idx(), Some((5, 1.to_string())));
    assert_eq!(iter.next_with_idx(), Some((6, 0.to_string())));
    assert_eq!(iter.next_with_idx(), Some((7, 1.to_string())));
    assert_eq!(iter.next_with_idx(), Some((8, 2.to_string())));
    assert_eq!(iter.next_with_idx(), Some((9, 0.to_string())));
    assert_eq!(iter.next_with_idx(), Some((10, 0.to_string())));
    assert_eq!(iter.next_with_idx(), Some((11, 0.to_string())));
    assert_eq!(iter.next_with_idx(), Some((12, 1.to_string())));
    assert_eq!(iter.next_with_idx(), Some((13, 0.to_string())));
    assert_eq!(iter.next_with_idx(), None);
    assert_eq!(iter.next_with_idx(), None);
    assert_eq!(iter.next_with_idx(), None);
    assert_eq!(iter.next_with_idx(), None);
}

#[test]
fn size_hint() {
    // 1 2 3 0 0 1 0 1 2 0 0 0 1 0
    let extend = |s: &String| {
        let i: usize = s.parse().unwrap();
        (0..i).map(|x| x.to_string())
    };
    let iter = ConcurrentRecursiveIterCrossbeam::new(vec(3), extend);

    // 1 2 3
    assert_eq!(iter.size_hint(), (3, None));

    _ = iter.next(); // 2 3 0
    assert_eq!(iter.size_hint(), (3, None));

    _ = iter.next(); // 3 0 0 1
    assert_eq!(iter.size_hint(), (4, None));

    _ = iter.next(); // 0 0 1 0 1 2
    assert_eq!(iter.size_hint(), (6, None));

    _ = iter.next(); // 0 1 0 1 2
    assert_eq!(iter.size_hint(), (5, None));

    _ = iter.next(); // 1 0 1 2
    assert_eq!(iter.size_hint(), (4, None));

    _ = iter.next(); // 0 1 2 0
    assert_eq!(iter.size_hint(), (4, None));

    _ = iter.next(); // 1 2 0
    assert_eq!(iter.size_hint(), (3, None));

    _ = iter.next(); // 2 0 0
    assert_eq!(iter.size_hint(), (3, None));

    _ = iter.next(); // 0 0 0 1
    assert_eq!(iter.size_hint(), (4, None));

    _ = iter.next(); // 0 0 1
    assert_eq!(iter.size_hint(), (3, None));

    _ = iter.next(); // 0 1
    assert_eq!(iter.size_hint(), (2, None));

    _ = iter.next(); // 1
    assert_eq!(iter.size_hint(), (1, None));

    _ = iter.next(); // 0
    assert_eq!(iter.size_hint(), (1, None));

    _ = iter.next(); // []
    assert_eq!(iter.size_hint(), (0, Some(0)));
}

#[test]
fn size_hint_skip_to_end() {
    // 1 2 3 0 0 1 0 1 2 0 0 0 1 0
    let extend = |s: &String| {
        let i: usize = s.parse().unwrap();
        (0..i).map(|x| x.to_string())
    };
    let iter = ConcurrentRecursiveIterCrossbeam::new(vec(3), extend);

    // 1 2 3
    assert_eq!(iter.size_hint(), (3, None));

    _ = iter.next(); // 2 3 0
    assert_eq!(iter.size_hint(), (3, None));

    _ = iter.next(); // 3 0 0 1
    assert_eq!(iter.size_hint(), (4, None));

    _ = iter.next(); // 0 0 1 0 1 2
    assert_eq!(iter.size_hint(), (6, None));

    iter.skip_to_end();
    assert_eq!(iter.size_hint(), (0, Some(0)));

    assert_eq!(iter.next(), None);
}

#[test_matrix([1, 2, 4])]
fn empty(nt: usize) {
    let extend = |s: &String| {
        let i: usize = s.parse().unwrap();
        (0..i).map(|x| x.to_string())
    };
    let iter = ConcurrentRecursiveIterCrossbeam::new(vec(0), extend);

    std::thread::scope(|s| {
        for _ in 0..nt {
            s.spawn(|| {
                assert!(iter.next().is_none());
                assert!(iter.next().is_none());

                let mut puller = iter.chunk_puller(5);
                assert!(puller.pull().is_none());
                assert!(puller.pull().is_none());

                let mut iter = iter.chunk_puller(5).flattened();
                assert!(iter.next().is_none());
                assert!(iter.next().is_none());
            });
        }
    });
}

fn extend<'a, 'b>(node: &'a &'b Node) -> core::slice::Iter<'b, Node> {
    node.children.iter()
}

fn assert_eq(roots: &Roots, bag: ConcurrentBag<&Node>) {
    let mut expected = Vec::new();
    expected.extend(roots.as_slice());
    let mut i = 0;
    while let Some(node) = expected.get(i) {
        expected.extend(node.children.iter());
        i += 1;
    }
    expected.sort();

    let mut collected = bag.into_inner().to_vec();
    collected.sort();

    assert_eq!(expected, collected);
}

fn assert_eq_with_idx(roots: &Roots, bag: ConcurrentBag<(usize, &Node)>) {
    let mut expected = Vec::new();
    expected.extend(roots.as_slice().iter().enumerate());
    let mut i = 0;
    while let Some((_, node)) = expected.get(i) {
        let len = expected.len();
        expected.extend(node.children.iter().enumerate().map(|(i, x)| (len + i, x)));
        i += 1;
    }

    let mut collected = bag.into_inner().to_vec();
    collected.sort();

    let mut idx1: Vec<_> = collected.iter().map(|x| x.0).collect();
    let idx2: Vec<_> = (0..collected.len()).collect();
    idx1.sort();
    assert_eq!(idx1, idx2);

    let mut nodes1: Vec<_> = collected.iter().map(|x| x.1).collect();
    let mut nodes2: Vec<_> = expected.iter().map(|x| x.1).collect();
    nodes1.sort();
    nodes2.sort();
    assert_eq!(nodes1, nodes2);
}

#[test_matrix([0, 1, N_ROOT], [1, 2, 4])]
fn next(n: usize, nt: usize) {
    let roots = Roots::new(n, N_NODE, 424242);
    let iter = ConcurrentRecursiveIterCrossbeam::new(roots.as_slice().iter(), extend);

    let bag = ConcurrentBag::new();
    let num_spawned = AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..nt {
            s.spawn(|| {
                // allow all threads to be spawned
                _ = num_spawned.fetch_add(1, Ordering::Relaxed);
                while num_spawned.load(Ordering::Relaxed) < nt {}

                while let Some(x) = iter.next() {
                    _ = iter.size_hint();
                    bag.push(x);
                }
            });
        }
    });

    assert_eq(&roots, bag);
}

#[test_matrix([0, 1, N], [1, 2, 4])]
fn next_with_idx(n: usize, nt: usize) {
    let roots = Roots::new(n, N_NODE, 3234);
    let iter = ConcurrentRecursiveIterCrossbeam::new(roots.as_slice().iter(), extend);

    let bag = ConcurrentBag::new();
    let num_spawned = AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..nt {
            s.spawn(|| {
                // allow all threads to be spawned
                _ = num_spawned.fetch_add(1, Ordering::Relaxed);
                while num_spawned.load(Ordering::Relaxed) < nt {}

                while let Some(x) = iter.next_with_idx() {
                    _ = iter.size_hint();
                    bag.push(x);
                }
            });
        }
    });

    assert_eq_with_idx(&roots, bag);
}

#[test_matrix([0, 1, N], [1, 2, 4])]
fn item_puller(n: usize, nt: usize) {
    let roots = Roots::new(n, N_NODE, 3234);
    let iter = ConcurrentRecursiveIterCrossbeam::new(roots.as_slice().iter(), extend);

    let bag = ConcurrentBag::new();
    let num_spawned = AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..nt {
            s.spawn(|| {
                // allow all threads to be spawned
                _ = num_spawned.fetch_add(1, Ordering::Relaxed);
                while num_spawned.load(Ordering::Relaxed) < nt {}

                for x in iter.item_puller() {
                    _ = iter.size_hint();
                    bag.push(x);
                }
            });
        }
    });

    assert_eq(&roots, bag);
}

#[test_matrix([0, 1, N], [1, 2, 4])]
fn item_puller_with_idx(n: usize, nt: usize) {
    let roots = Roots::new(n, N_NODE, 3234);
    let iter = ConcurrentRecursiveIterCrossbeam::new(roots.as_slice().iter(), extend);

    let bag = ConcurrentBag::new();
    let num_spawned = AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..nt {
            s.spawn(|| {
                // allow all threads to be spawned
                _ = num_spawned.fetch_add(1, Ordering::Relaxed);
                while num_spawned.load(Ordering::Relaxed) < nt {}

                for x in iter.item_puller_with_idx() {
                    _ = iter.size_hint();
                    bag.push(x);
                }
            });
        }
    });

    assert_eq_with_idx(&roots, bag);
}

// #[test_matrix([0, 1, N], [1, 2, 4])]
fn chunk_puller(n: usize, nt: usize) {
    let roots = Roots::new(n, N_NODE, 3234);
    let iter = ConcurrentRecursiveIterCrossbeam::new(roots.as_slice().iter(), extend);

    let bag = ConcurrentBag::new();
    let num_spawned = AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..nt {
            s.spawn(|| {
                // allow all threads to be spawned
                _ = num_spawned.fetch_add(1, Ordering::Relaxed);
                while num_spawned.load(Ordering::Relaxed) < nt {}

                let mut puller = iter.chunk_puller(7);

                while let Some(chunk) = puller.pull() {
                    assert!(chunk.len() <= 7);
                    for x in chunk {
                        bag.push(x);
                    }
                }
            });
        }
    });

    assert_eq(&roots, bag);
}

// #[test_matrix([0, 1, N], [1, 2, 4])]
fn chunk_puller_with_idx(n: usize, nt: usize) {
    let roots = Roots::new(n, N_NODE, 3234);
    let iter = ConcurrentRecursiveIterCrossbeam::new(roots.as_slice().iter(), extend);

    let bag = ConcurrentBag::new();
    let num_spawned = AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..nt {
            s.spawn(|| {
                // allow all threads to be spawned
                _ = num_spawned.fetch_add(1, Ordering::Relaxed);
                while num_spawned.load(Ordering::Relaxed) < nt {}

                let mut puller = iter.chunk_puller(7);

                while let Some((begin_idx, chunk)) = puller.pull_with_idx() {
                    assert!(chunk.len() <= 7);
                    for (i, x) in chunk.enumerate() {
                        bag.push((begin_idx + i, x));
                    }
                }
            });
        }
    });

    assert_eq_with_idx(&roots, bag);
}

// #[test_matrix([0, 1, N], [1, 2, 4])]
fn flattened_chunk_puller(n: usize, nt: usize) {
    let roots = Roots::new(n, N_NODE, 3234);
    let iter = ConcurrentRecursiveIterCrossbeam::new(roots.as_slice().iter(), extend);

    let bag = ConcurrentBag::new();
    let num_spawned = AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..nt {
            s.spawn(|| {
                // allow all threads to be spawned
                _ = num_spawned.fetch_add(1, Ordering::Relaxed);
                while num_spawned.load(Ordering::Relaxed) < nt {}

                for x in iter.chunk_puller(7).flattened() {
                    bag.push(x);
                }
            });
        }
    });

    assert_eq(&roots, bag);
}

// #[test_matrix([0, 1, N], [1, 2, 4])]
fn flattened_chunk_puller_with_idx(n: usize, nt: usize) {
    let roots = Roots::new(n, N_NODE, 3234);
    let iter = ConcurrentRecursiveIterCrossbeam::new(roots.as_slice().iter(), extend);

    let bag = ConcurrentBag::new();
    let num_spawned = AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..nt {
            s.spawn(|| {
                // allow all threads to be spawned
                _ = num_spawned.fetch_add(1, Ordering::Relaxed);
                while num_spawned.load(Ordering::Relaxed) < nt {}

                for x in iter.chunk_puller(7).flattened_with_idx() {
                    bag.push(x);
                }
            });
        }
    });

    assert_eq_with_idx(&roots, bag);
}

#[test_matrix([0, 1, N], [1, 2, 4])]
fn skip_to_end(n: usize, nt: usize) {
    let roots = Roots::new(n, N_NODE, 3234);
    let iter = ConcurrentRecursiveIterCrossbeam::new(roots.as_slice().iter(), extend);

    let until = n / 2;

    let bag = ConcurrentBag::new();
    let num_spawned = AtomicUsize::new(0);
    let con_num_spawned = &num_spawned;
    let con_bag = &bag;
    let con_iter = &iter;
    std::thread::scope(|s| {
        for t in 0..nt {
            s.spawn(move || {
                // allow all threads to be spawned
                _ = con_num_spawned.fetch_add(1, Ordering::Relaxed);
                while con_num_spawned.load(Ordering::Relaxed) < nt {}

                match t % 2 {
                    0 => {
                        while let Some((idx, node)) = con_iter.next_with_idx() {
                            match idx < until {
                                true => _ = con_bag.push(node),
                                false => con_iter.skip_to_end(),
                            }
                        }
                    }
                    _ => {
                        for (idx, node) in con_iter.chunk_puller(7).flattened_with_idx() {
                            match idx < until {
                                true => _ = con_bag.push(node),
                                false => con_iter.skip_to_end(),
                            }
                        }
                    }
                }
            });
        }
    });

    let mut expected_super_set = Vec::new();
    expected_super_set.extend(roots.as_slice());
    let mut i = 0;
    while let Some(node) = expected_super_set.get(i) {
        expected_super_set.extend(node.children.iter());
        i += 1;
        if i > until {
            break;
        }
    }
    expected_super_set.sort();

    let mut collected = bag.into_inner().to_vec();
    collected.sort();

    for x in collected {
        assert!(expected_super_set.contains(&x));
    }
}

#[test_matrix([0, 1, N], [1, 2, 4], [0, N / 2, N])]
fn into_seq_iter(n: usize, nt: usize, until: usize) {
    let roots = Roots::new(n, N_NODE, 3234);
    let iter = ConcurrentRecursiveIterCrossbeam::new(roots.as_slice().iter(), extend);

    let bag = ConcurrentBag::new();
    let num_spawned = AtomicUsize::new(0);
    let con_num_spawned = &num_spawned;
    let con_bag = &bag;
    let con_iter = &iter;
    if until > 0 {
        std::thread::scope(|s| {
            for t in 0..nt {
                s.spawn(move || {
                    // allow all threads to be spawned
                    _ = con_num_spawned.fetch_add(1, Ordering::Relaxed);
                    while con_num_spawned.load(Ordering::Relaxed) < nt {}

                    match t % 2 {
                        0 => {
                            while let Some((idx, node)) = con_iter.next_with_idx() {
                                con_bag.push(node);
                                if idx >= until {
                                    break;
                                }
                            }
                        }
                        _ => {
                            let mut iter = con_iter.chunk_puller(7);
                            while let Some((begin_idx, chunk)) = iter.pull_with_idx() {
                                let mut do_break = false;
                                for (i, node) in chunk.into_iter().enumerate() {
                                    con_bag.push(node);
                                    let idx = begin_idx + i;
                                    if idx >= until {
                                        do_break = true;
                                    }
                                }
                                if do_break {
                                    break;
                                }
                            }
                        }
                    }
                });
            }
        });
    }

    let iter = iter.into_seq_iter();
    let remaining: Vec<_> = iter.collect();
    let collected = bag.into_inner().to_vec();
    let mut all: Vec<_> = collected.into_iter().chain(remaining).collect();
    all.sort();

    let mut expected = Vec::new();
    expected.extend(roots.as_slice());
    let mut i = 0;
    while let Some(node) = expected.get(i) {
        expected.extend(node.children.iter());
        i += 1;
    }
    expected.sort();

    assert_eq!(all, expected);
}
