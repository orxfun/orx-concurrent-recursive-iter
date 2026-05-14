use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use orx_concurrent_iter::ConcurrentIter;
use orx_concurrent_recursive_iter::{
    ConcurrentRecursiveIter, ConcurrentRecursiveIterShards, Queue,
};
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use rayon::{Scope, ThreadPool, ThreadPoolBuilder, scope};
use std::hint::black_box;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

const THREADS: [usize; 4] = [8, 16, 24, 32];

#[derive(Clone)]
struct DirNode {
    id: usize,
    file_count: usize,
    children: Vec<usize>,
}

impl DirNode {
    fn compute_score(&self, work: usize) -> u64 {
        (0..work)
            .map(|j| {
                let n = black_box(((self.id + self.file_count + j) % 35) as u64);
                let mut a = 0u64;
                let mut b = 1u64;
                for _ in 0..n {
                    let c = black_box(a + b);
                    a = b;
                    b = c;
                }
                a
            })
            .sum()
    }
}

#[derive(Clone)]
struct FileSystem {
    roots: Vec<usize>,
    nodes: Vec<DirNode>,
}

impl FileSystem {
    fn generate(num_nodes: usize, num_roots: usize, max_children: usize, seed: u64) -> Self {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let num_nodes = num_nodes.max(1);
        let num_roots = num_roots.min(num_nodes).max(1);
        let max_children = max_children.max(1);

        let mut nodes: Vec<_> = (0..num_nodes)
            .map(|id| DirNode {
                id,
                file_count: rng.random_range(1..20),
                children: Vec::new(),
            })
            .collect();

        let roots: Vec<usize> = (0..num_roots).collect();
        let mut open_parents: Vec<usize> = (0..num_roots).collect();

        for child in num_roots..num_nodes {
            if open_parents.is_empty() {
                open_parents.push(rng.random_range(0..child));
            }

            let parent_slot = rng.random_range(0..open_parents.len());
            let parent = open_parents[parent_slot];

            nodes[parent].children.push(child);
            if nodes[parent].children.len() >= max_children {
                open_parents.swap_remove(parent_slot);
            }

            open_parents.push(child);
        }

        Self { roots, nodes }
    }
}

fn seq_sum(fs: &FileSystem, work: usize) -> u64 {
    let mut stack = fs.roots.clone();
    let mut sum = 0u64;

    while let Some(idx) = stack.pop() {
        let node = &fs.nodes[idx];
        sum += node.compute_score(work);
        stack.extend(node.children.iter().copied());
    }

    sum
}

fn rayon_sum(fs: &FileSystem, work: usize, pool: &ThreadPool) -> u64 {
    fn spawn_job<'a>(
        scope: &Scope<'a>,
        fs: &'a FileSystem,
        idx: usize,
        work: usize,
        sum: &'a AtomicU64,
    ) {
        scope.spawn(move |scope| {
            let node = &fs.nodes[idx];
            for child in node.children.iter().copied() {
                spawn_job(scope, fs, child, work, sum);
            }
            sum.fetch_add(node.compute_score(work), Ordering::Relaxed);
        });
    }

    let sum = AtomicU64::new(0);
    pool.install(|| {
        scope(|scope| {
            for root in fs.roots.iter().copied() {
                spawn_job(scope, fs, root, work, &sum);
            }
        });
    });

    sum.load(Ordering::Relaxed)
}

fn recursive_iter_sum(fs: &FileSystem, work: usize, num_threads: usize) -> u64 {
    let iter = ConcurrentRecursiveIter::new_exact(
        fs.roots.iter().copied(),
        |idx: &usize, queue: &Queue<usize>| {
            queue.extend(fs.nodes[*idx].children.iter().copied());
        },
        fs.nodes.len(),
    );

    let num_threads = num_threads.max(1);
    let num_spawned = AtomicUsize::new(0);

    std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(num_threads);
        for _ in 0..num_threads {
            handles.push(scope.spawn(|| {
                num_spawned.fetch_add(1, Ordering::Relaxed);
                while num_spawned.load(Ordering::Relaxed) < num_threads {
                    core::hint::spin_loop();
                }

                let mut local_sum = 0u64;
                while let Some(idx) = iter.next() {
                    local_sum += fs.nodes[idx].compute_score(work);
                }
                local_sum
            }));
        }

        handles.into_iter().map(|h| h.join().unwrap()).sum()
    })
}

fn recursive_iter_shards_sum(
    fs: &FileSystem,
    work: usize,
    num_threads: usize,
    num_shards: usize,
) -> u64 {
    let num_shards = NonZeroUsize::new(num_shards)
        .unwrap_or_else(|| panic!("num_shards must be greater than zero"));

    let iter = ConcurrentRecursiveIterShards::new_exact_with_shards(
        fs.roots.iter().copied(),
        |idx: &usize, queue| {
            queue.extend(fs.nodes[*idx].children.iter().copied());
        },
        fs.nodes.len(),
        num_shards,
    );

    let num_threads = num_threads.max(1);
    let num_spawned = AtomicUsize::new(0);

    std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(num_threads);
        for _ in 0..num_threads {
            handles.push(scope.spawn(|| {
                num_spawned.fetch_add(1, Ordering::Relaxed);
                while num_spawned.load(Ordering::Relaxed) < num_threads {
                    core::hint::spin_loop();
                }

                let mut local_sum = 0u64;
                while let Some(idx) = iter.next() {
                    local_sum += fs.nodes[idx].compute_score(work);
                }
                local_sum
            }));
        }

        handles.into_iter().map(|h| h.join().unwrap()).sum()
    })
}

fn recursive_iter_scaling(c: &mut Criterion) {
    let nodes = 40_000;
    let roots = 100;
    let max_children = 8;
    let work = 300;
    let seed = 42;

    let fs = FileSystem::generate(nodes, roots, max_children, seed);
    let expected = seq_sum(&fs, work);

    let mut seq_group = c.benchmark_group("recursive-iter/seq");
    seq_group.sample_size(10);
    seq_group.bench_function("sequential", |b| {
        b.iter(|| {
            let value = seq_sum(&fs, work);
            assert_eq!(value, expected);
            black_box(value);
        });
    });
    seq_group.finish();

    let mut rayon_group = c.benchmark_group("recursive-iter/rayon");
    rayon_group.sample_size(10);
    for threads in THREADS {
        let pool = ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap_or_else(|e| panic!("failed to build rayon pool: {e}"));

        rayon_group.bench_with_input(BenchmarkId::from_parameter(threads), &threads, |b, _| {
            b.iter(|| {
                let value = rayon_sum(&fs, work, &pool);
                assert_eq!(value, expected);
                black_box(value);
            });
        });
    }
    rayon_group.finish();

    let mut reciter_group = c.benchmark_group("recursive-iter/original");
    reciter_group.sample_size(10);
    for threads in THREADS {
        reciter_group.bench_with_input(BenchmarkId::from_parameter(threads), &threads, |b, &t| {
            b.iter(|| {
                let value = recursive_iter_sum(&fs, work, t);
                assert_eq!(value, expected);
                black_box(value);
            });
        });
    }
    reciter_group.finish();

    let mut shards_group = c.benchmark_group("recursive-iter/sharded");
    shards_group.sample_size(10);
    for threads in THREADS {
        let shards = threads;
        shards_group.bench_with_input(
            BenchmarkId::new("threads-shards", format!("{threads}-{shards}")),
            &(threads, shards),
            |b, &(t, s)| {
                b.iter(|| {
                    let value = recursive_iter_shards_sum(&fs, work, t, s);
                    assert_eq!(value, expected);
                    black_box(value);
                });
            },
        );
    }
    shards_group.finish();

    let mut shards_div8_group = c.benchmark_group("recursive-iter/sharded-div8");
    shards_div8_group.sample_size(10);
    for threads in THREADS {
        let shards = (threads / 8).max(1);
        shards_div8_group.bench_with_input(
            BenchmarkId::new("threads-shards", format!("{threads}-{shards}")),
            &(threads, shards),
            |b, &(t, s)| {
                b.iter(|| {
                    let value = recursive_iter_shards_sum(&fs, work, t, s);
                    assert_eq!(value, expected);
                    black_box(value);
                });
            },
        );
    }
    shards_div8_group.finish();
}

criterion_group!(benches, recursive_iter_scaling);
criterion_main!(benches);
