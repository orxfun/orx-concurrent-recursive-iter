use criterion::{Criterion, criterion_group, criterion_main};
use orx_concurrent_iter::{ChunkPuller, ConcurrentIter};
use orx_concurrent_recursive_iter::{
    ConcurrentRecursiveIter, ConcurrentRecursiveIterCrossbeam,
    ConcurrentRecursiveIterCrossbeamNoStd, Queue,
};
use orx_criterion::{Experiment, Factors};
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use rayon::{ThreadPool, ThreadPoolBuilder, scope};
use std::sync::atomic::{AtomicU64, Ordering};

// const THREADS: [usize; 4] = [8, 16, 24, 32];
const THREADS: [usize; 2] = [16, 32];
const CHUNK_SIZE: usize = 64;

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
        match chunk_size {
            1 => {
                while let Some(idx) = iter.next_by(thread_idx) {
                    local_sum += fs.nodes[idx].compute_score(work);
                }
            }
            c => {
                let mut puller = iter.chunk_puller_by(c, thread_idx);
                while let Some(chunk) = puller.pull() {
                    local_sum += chunk
                        .into_iter()
                        .map(|idx| fs.nodes[idx].compute_score(work))
                        .sum::<u64>();
                }
            }
        }

        local_sum
    })
    .into_iter()
    .sum()
}

fn concurrent_recursive_iter_sum(
    fs: &FileSystem,
    work: usize,
    pool: &ThreadPool,
    chunk_size: usize,
) -> u64 {
    let iter = ConcurrentRecursiveIter::new_exact(
        fs.roots.iter().copied(),
        |idx: &usize, queue: &Queue<usize>| {
            queue.extend(fs.nodes[*idx].children.iter().copied());
        },
        fs.nodes.len(),
    );

    run_concurrent_iter(&iter, fs, work, pool, chunk_size)
}

fn cross_std_sum(fs: &FileSystem, work: usize, pool: &ThreadPool, chunk_size: usize) -> u64 {
    let iter = ConcurrentRecursiveIterCrossbeam::new(
        fs.roots.iter().copied(),
        |idx: &usize| fs.nodes[*idx].children.iter().copied(),
        Some(fs.nodes.len()),
        Some(pool.current_num_threads()),
    );

    run_concurrent_iter(&iter, fs, work, pool, chunk_size)
}

fn cross_no_std_sum(fs: &FileSystem, work: usize, pool: &ThreadPool, chunk_size: usize) -> u64 {
    let iter = ConcurrentRecursiveIterCrossbeamNoStd::new(
        fs.roots.iter().copied(),
        |idx: &usize| fs.nodes[*idx].children.iter().copied(),
        Some(fs.nodes.len()),
        Some(pool.current_num_threads()),
    );

    run_concurrent_iter(&iter, fs, work, pool, chunk_size)
}

#[derive(Clone)]
struct Input {
    num_threads: usize,
    nodes: usize,
    roots: usize,
    max_children: usize,
    work: usize,
    seed: u64,
}

impl Factors for Input {
    fn factor_names() -> Vec<&'static str> {
        vec!["threads", "nodes", "roots", "work"]
    }

    fn factor_levels(&self) -> Vec<String> {
        vec![
            self.num_threads.to_string(),
            self.nodes.to_string(),
            self.roots.to_string(),
            self.work.to_string(),
        ]
    }

    fn factor_levels_short(&self) -> Vec<String> {
        vec![
            self.num_threads.to_string(),
            self.nodes.to_string(),
            self.roots.to_string(),
            self.work.to_string(),
        ]
    }
}

#[derive(Debug, Clone, Copy)]
enum Method {
    Seq,
    Rayon,
    CrossStd,
    CrossStdChunk,
    CrossNoStd,
    CrossNoStdChunk,
    RecIter,
    RecIterChunk,
}

impl Factors for Method {
    fn factor_names() -> Vec<&'static str> {
        vec!["method"]
    }

    fn factor_levels(&self) -> Vec<String> {
        vec![
            match self {
                Self::Seq => "seq",
                Self::Rayon => "rayon",
                Self::CrossStd => "cross-std",
                Self::CrossStdChunk => "cross-std-c64",
                Self::CrossNoStd => "cross-no-std",
                Self::CrossNoStdChunk => "cross-no-std-c64",
                Self::RecIter => "orx",
                Self::RecIterChunk => "orx-c64",
            }
            .to_string(),
        ]
    }

    fn factor_levels_short(&self) -> Vec<String> {
        vec![
            match self {
                Self::Seq => "s",
                Self::Rayon => "r",
                Self::CrossStd => "cs",
                Self::CrossStdChunk => "cs-c64",
                Self::CrossNoStd => "cns",
                Self::CrossNoStdChunk => "cns-c64",
                Self::RecIter => "o",
                Self::RecIterChunk => "o-c64",
            }
            .to_string(),
        ]
    }
}

struct Exp;

impl Experiment for Exp {
    type InputFactors = Input;
    type AlgFactors = Method;
    type Input = (ThreadPool, FileSystem);
    type Output = u64;

    fn input(&mut self, input_variant: &Self::InputFactors) -> Self::Input {
        let pool = ThreadPoolBuilder::new()
            .num_threads(input_variant.num_threads)
            .build()
            .unwrap_or_else(|e| panic!("failed to build rayon pool: {e}"));
        let fs = FileSystem::generate(
            input_variant.nodes,
            input_variant.roots,
            input_variant.max_children,
            input_variant.seed,
        );
        (pool, fs)
    }

    fn execute(
        &mut self,
        input_variant: &Self::InputFactors,
        alg_variant: &Self::AlgFactors,
        input: &Self::Input,
    ) -> Self::Output {
        let (pool, fs) = input;
        match alg_variant {
            Method::Seq => seq_sum(fs, input_variant.work),
            Method::Rayon => rayon_sum(fs, input_variant.work, pool),
            Method::CrossStd => cross_std_sum(fs, input_variant.work, pool, 1),
            Method::CrossStdChunk => cross_std_sum(fs, input_variant.work, pool, CHUNK_SIZE),
            Method::CrossNoStd => cross_no_std_sum(fs, input_variant.work, pool, 1),
            Method::CrossNoStdChunk => cross_no_std_sum(fs, input_variant.work, pool, CHUNK_SIZE),
            Method::RecIter => concurrent_recursive_iter_sum(fs, input_variant.work, pool, 1),
            Method::RecIterChunk => {
                concurrent_recursive_iter_sum(fs, input_variant.work, pool, CHUNK_SIZE)
            }
        }
    }

    fn validate_output(
        &self,
        input_variant: &Self::InputFactors,
        input: &Self::Input,
        output: &Self::Output,
    ) {
        let (_, fs) = input;
        let expected = seq_sum(fs, input_variant.work);
        assert_eq!(expected, *output);
    }
}

fn run(c: &mut Criterion) {
    let work = [1, 100, 400, 1600];
    let treatments: Vec<_> = THREADS
        .iter()
        .copied()
        .flat_map(|num_threads| {
            work.into_iter().map(move |work| Input {
                num_threads,
                nodes: 40_000,
                roots: 100,
                max_children: 8,
                work,
                seed: 42,
            })
        })
        .collect();

    let variants = vec![
        // Method::Seq,
        Method::Rayon,
        Method::CrossStd,
        Method::CrossStdChunk,
        Method::CrossNoStd,
        Method::CrossNoStdChunk,
        Method::RecIter,
        Method::RecIterChunk,
    ];

    Exp.bench(c, "ben", &treatments, &variants);
}

criterion_group!(benches, run);
criterion_main!(benches);
