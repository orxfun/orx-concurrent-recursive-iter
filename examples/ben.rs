use orx_concurrent_iter::ConcurrentIter;
use orx_concurrent_recursive_iter::{Con1, ConcurrentRecursiveIter, Queue};
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use rayon::{ThreadPool, ThreadPoolBuilder, scope};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

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
                let n = core::hint::black_box(((self.id + self.file_count + j) % 35) as u64);
                let mut a = 0u64;
                let mut b = 1u64;
                for _ in 0..n {
                    let c = core::hint::black_box(a + b);
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
        scope: &rayon::Scope<'a>,
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

fn run_concurrent_iter<I>(iter: &I, fs: &FileSystem, work: usize, num_threads: usize) -> u64
where
    I: ConcurrentIter<Item = usize> + Sync,
{
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

fn concurrent_recursive_iter_sum(fs: &FileSystem, work: usize, num_threads: usize) -> u64 {
    let iter = ConcurrentRecursiveIter::new_exact(
        fs.roots.iter().copied(),
        |idx: &usize, queue: &Queue<usize>| {
            queue.extend(fs.nodes[*idx].children.iter().copied());
        },
        fs.nodes.len(),
    );

    run_concurrent_iter(&iter, fs, work, num_threads)
}

fn con1_sum(fs: &FileSystem, work: usize, num_threads: usize) -> u64 {
    let iter = Con1::new_exact(
        fs.roots.iter().copied(),
        |idx: &usize| fs.nodes[*idx].children.clone(),
        fs.nodes.len(),
    );

    run_concurrent_iter(&iter, fs, work, num_threads)
}

#[derive(Clone, Copy, Debug)]
enum Method {
    Seq,
    Rayon,
    Con1,
    RecIter,
}

impl Method {
    fn all() -> [Self; 4] {
        [Self::Seq, Self::Rayon, Self::Con1, Self::RecIter]
    }

    fn label(self) -> &'static str {
        match self {
            Self::Seq => "seq",
            Self::Rayon => "rayon",
            Self::Con1 => "con1",
            Self::RecIter => "orx",
        }
    }
}

fn main() {
    let num_threads = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1);

    let fs = FileSystem::generate(40_000, 100, 8, 42);
    let work = 300;
    let expected = seq_sum(&fs, work);
    let pool = ThreadPoolBuilder::new()
        .num_threads(num_threads)
        .build()
        .unwrap_or_else(|e| panic!("failed to build rayon pool: {e}"));

    for method in Method::all() {
        let started = Instant::now();
        let output = match method {
            Method::Seq => seq_sum(&fs, work),
            Method::Rayon => rayon_sum(&fs, work, &pool),
            Method::Con1 => con1_sum(&fs, work, num_threads),
            Method::RecIter => concurrent_recursive_iter_sum(&fs, work, num_threads),
        };
        let elapsed = started.elapsed();

        assert_eq!(expected, output);
        println!("{:<6} {:>10?}  sum={}", method.label(), elapsed, output);
    }
}
