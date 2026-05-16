use crate::ConcurrentRecursiveIterShards2;
use alloc::{
    vec,
    vec::Vec,
};
use core::sync::atomic::{AtomicU64, Ordering};
use orx_concurrent_iter::{ChunkPuller, ConcurrentIter};
use test_case::test_case;

#[derive(Clone)]
struct Fs {
    roots: Vec<usize>,
    children: Vec<Vec<usize>>,
}

impl Fs {
    fn binary_tree(num_nodes: usize) -> Self {
        let mut children = vec![Vec::new(); num_nodes];
        for (i, slot) in children.iter_mut().enumerate().take(num_nodes) {
            let c1 = 2 * i + 1;
            let c2 = 2 * i + 2;
            if c1 < num_nodes {
                slot.push(c1);
            }
            if c2 < num_nodes {
                slot.push(c2);
            }
        }

        Self {
            roots: vec![0],
            children,
        }
    }

    fn expected_sum(&self) -> u64 {
        (0..self.children.len()).map(score).sum()
    }
}

#[inline(always)]
fn score(idx: usize) -> u64 {
    let n = ((idx % 31) + 1) as u64;
    let mut a = 0u64;
    let mut b = 1u64;
    for _ in 0..n {
        let c = a + b;
        a = b;
        b = c;
    }
    a
}

#[test_case(1; "shard1-next")]
#[test_case(2; "shard2-next")]
fn shards2_next_consumes_all(shards: usize) {
    let fs = Fs::binary_tree(2047);
    let expected = fs.expected_sum();

    let iter = ConcurrentRecursiveIterShards2::new_exact_with_shards(
        fs.roots.iter().copied(),
        |idx: &usize, queue| {
            queue.extend(fs.children[*idx].iter().copied());
        },
        fs.children.len(),
        shards,
    );

    let num_threads = 8;
    let total = AtomicU64::new(0);

    std::thread::scope(|s| {
        for _ in 0..num_threads {
            s.spawn(|| {
                let mut local = 0u64;
                while let Some(idx) = iter.next() {
                    local += score(idx);
                }
                total.fetch_add(local, Ordering::Relaxed);
            });
        }
    });

    assert_eq!(expected, total.into_inner());
}

#[test_case(1; "shard1-chunk")]
#[test_case(2; "shard2-chunk")]
fn shards2_chunk_consumes_all(shards: usize) {
    let fs = Fs::binary_tree(2047);
    let expected = fs.expected_sum();

    let iter = ConcurrentRecursiveIterShards2::new_exact_with_shards(
        fs.roots.iter().copied(),
        |idx: &usize, queue| {
            queue.extend(fs.children[*idx].iter().copied());
        },
        fs.children.len(),
        shards,
    );

    let num_threads = 8;
    let total = AtomicU64::new(0);

    std::thread::scope(|s| {
        for _ in 0..num_threads {
            s.spawn(|| {
                let mut local = 0u64;
                let mut puller = iter.chunk_puller(64);
                while let Some(chunk) = puller.pull() {
                    local += chunk.into_iter().map(score).sum::<u64>();
                }
                total.fetch_add(local, Ordering::Relaxed);
            });
        }
    });

    assert_eq!(expected, total.into_inner());
}
