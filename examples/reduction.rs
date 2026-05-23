use clap::Parser;
use orx_concurrent_iter::{ChunkPuller, ConcurrentIter};
use orx_concurrent_recursive_iter::ConcurrentRecursiveIter;
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use rayon::{ThreadPool, ThreadPoolBuilder, scope};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

#[derive(Debug, Parser)]
#[command(
    name = "ben",
    about = "Run recursive iteration methods with configurable threads and chunk size"
)]
struct Args {
    /// Number of worker threads used by parallel methods.
    #[arg(long = "num-threads")]
    num_threads: Option<usize>,

    /// Chunk size used by con1/orx base variants.
    #[arg(long = "chunk-size", default_value_t = 1)]
    chunk_size: usize,

    /// Amount of work
    #[arg(long = "work", default_value_t = 300)]
    work: usize,

    /// Number of warmup runs
    #[arg(long = "warm-up", default_value_t = 4)]
    warm_up: usize,
}

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

fn run_concurrent_iter<I>(
    iter: &I,
    fs: &FileSystem,
    work: usize,
    pool: &ThreadPool,
    chunk_size: usize,
) -> u64
where
    I: ConcurrentIter<Item = usize>,
{
    let chunk_size = chunk_size.max(1);

    pool.broadcast(|ctx| {
        let thread_idx = ctx.index();
        let mut local_sum = 0u64;
        if chunk_size == 1 {
            while let Some(idx) = iter.next_by(thread_idx) {
                local_sum += fs.nodes[idx].compute_score(work);
            }
        } else {
            let mut puller = iter.chunk_puller_by(chunk_size, thread_idx);
            while let Some(chunk) = puller.pull() {
                local_sum += chunk
                    .into_iter()
                    .map(|idx| fs.nodes[idx].compute_score(work))
                    .sum::<u64>();
            }
        }
        local_sum
    })
    .into_iter()
    .sum()
}

fn orx(fs: &FileSystem, work: usize, pool: &ThreadPool, chunk_size: usize) -> u64 {
    let iter = ConcurrentRecursiveIter::new(
        fs.roots.iter().copied(),
        |idx: &usize| fs.nodes[*idx].children.iter().copied(),
        Some(fs.nodes.len()),
        Some(pool.current_num_threads()),
    );

    run_concurrent_iter(&iter, fs, work, pool, chunk_size)
}

#[cfg(feature = "experimental")]
fn orx_queue(fs: &FileSystem, work: usize, pool: &ThreadPool, chunk_size: usize) -> u64 {
    use orx_concurrent_recursive_iter::{ConcurrentRecursiveIterQueue, Queue};

    let iter = ConcurrentRecursiveIterQueue::new_exact(
        fs.roots.iter().copied(),
        |idx: &usize, q: &Queue<'_, _>| q.extend(fs.nodes[*idx].children.iter().copied()),
        fs.nodes.len(),
    );

    run_concurrent_iter(&iter, fs, work, pool, chunk_size)
}

#[derive(Clone, Copy, Debug)]
enum Method {
    Seq,
    Rayon,
    Orx,
    #[cfg(feature = "experimental")]
    OrxQueue,
}

impl Method {
    fn all() -> [Self; 3] {
        [Self::Seq, Self::Rayon, Self::Orx]
    }

    fn label(self) -> &'static str {
        match self {
            Self::Seq => "seq",
            Self::Rayon => "rayon",
            Self::Orx => "orx",
            #[cfg(feature = "experimental")]
            Self::OrxQueue => "orx-queue",
        }
    }
}

fn run(
    methods: &[Method],
    fs: &FileSystem,
    work: usize,
    pool: &ThreadPool,
    chunk_size: usize,
    expected: u64,
) -> Vec<(Method, Duration)> {
    let mut rows = Vec::with_capacity(Method::all().len());

    for method in methods {
        let started = Instant::now();
        let output = match method {
            Method::Seq => seq_sum(fs, work),
            Method::Rayon => rayon_sum(fs, work, pool),
            Method::Orx => orx(fs, work, pool, chunk_size),
            #[cfg(feature = "experimental")]
            Method::OrxQueue => orx_queue(fs, work, pool, chunk_size),
        };
        let elapsed = started.elapsed();

        assert_eq!(expected, output, "{method:?}");
        rows.push((*method, elapsed));
    }

    rows
}

fn main() {
    let args = Args::parse();

    let default_threads = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1);
    let num_threads = args.num_threads.unwrap_or(default_threads).max(1);
    let chunk_size = args.chunk_size.max(1);
    let work = args.work.max(1);

    let fs = FileSystem::generate(100_000, 100, 64, 42);
    let expected = seq_sum(&fs, work);
    let pool = ThreadPoolBuilder::new()
        .num_threads(num_threads)
        .build()
        .unwrap_or_else(|e| panic!("failed to build rayon pool: {e}"));

    #[cfg(not(feature = "experimental"))]
    let methods = vec![Method::Seq, Method::Rayon, Method::Orx];
    #[cfg(feature = "experimental")]
    let methods = vec![Method::Seq, Method::Rayon, Method::Orx, Method::OrxQueue];

    for _ in 0..args.warm_up {
        _ = run(&methods, &fs, work, &pool, chunk_size, expected);
    }

    let rows = run(&methods, &fs, work, &pool, chunk_size, expected);

    let max_nanos = rows
        .iter()
        .map(|(_, elapsed)| elapsed.as_nanos())
        .max()
        .unwrap_or(0);

    for (method, elapsed) in rows {
        let bar_len = match max_nanos {
            0 => 0,
            n => ((elapsed.as_nanos() * 40 + (n / 2)) / n) as usize,
        };
        let bar = "▆".repeat(bar_len);
        println!("{:<10} {:>10?}\t{}", method.label(), elapsed, bar);
    }
}
