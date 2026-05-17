use criterion::{Criterion, criterion_group, criterion_main};
use orx_concurrent_iter::{ChunkPuller, ConcurrentIter};
use orx_concurrent_recursive_iter::{Con1, ConcurrentRecursiveIter, Queue};
use orx_criterion::{Experiment, Factors};
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use rayon::{ThreadPool, ThreadPoolBuilder, scope};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

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
    num_threads: usize,
    chunk_size: usize,
) -> u64
where
    I: ConcurrentIter<Item = usize> + Sync,
{
    let num_threads = num_threads.max(1);
    let chunk_size = chunk_size.max(1);
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
                match chunk_size {
                    1 => {
                        while let Some(idx) = iter.next() {
                            local_sum += fs.nodes[idx].compute_score(work);
                        }
                    }
                    c => {
                        let mut puller = iter.chunk_puller(c);
                        while let Some(chunk) = puller.pull() {
                            local_sum += chunk
                                .into_iter()
                                .map(|idx| fs.nodes[idx].compute_score(work))
                                .sum::<u64>();
                        }
                    }
                }

                local_sum
            }));
        }

        handles.into_iter().map(|h| h.join().unwrap()).sum()
    })
}

fn concurrent_recursive_iter_sum(
    fs: &FileSystem,
    work: usize,
    num_threads: usize,
    chunk_size: usize,
) -> u64 {
    let iter = ConcurrentRecursiveIter::new_exact(
        fs.roots.iter().copied(),
        |idx: &usize, queue: &Queue<usize>| {
            queue.extend(fs.nodes[*idx].children.iter().copied());
        },
        fs.nodes.len(),
    );

    run_concurrent_iter(&iter, fs, work, num_threads, chunk_size)
}

fn con1_sum(fs: &FileSystem, work: usize, num_threads: usize, chunk_size: usize) -> u64 {
    let iter = Con1::new_exact(
        fs.roots.iter().copied(),
        |idx: &usize| fs.nodes[*idx].children.clone(),
        fs.nodes.len(),
    );

    run_concurrent_iter(&iter, fs, work, num_threads, chunk_size)
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
    Con1,
    Con1Chunk,
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
                Self::Con1 => "con1",
                Self::Con1Chunk => "con1-c64",
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
                Self::Con1 => "c1",
                Self::Con1Chunk => "c1-c64",
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
    type Input = FileSystem;
    type Output = u64;

    fn input(&mut self, input_variant: &Self::InputFactors) -> Self::Input {
        FileSystem::generate(
            input_variant.nodes,
            input_variant.roots,
            input_variant.max_children,
            input_variant.seed,
        )
    }

    fn execute(
        &mut self,
        input_variant: &Self::InputFactors,
        alg_variant: &Self::AlgFactors,
        input: &Self::Input,
    ) -> Self::Output {
        match alg_variant {
            Method::Seq => seq_sum(input, input_variant.work),
            Method::Rayon => {
                let pool = ThreadPoolBuilder::new()
                    .num_threads(input_variant.num_threads)
                    .build()
                    .unwrap_or_else(|e| panic!("failed to build rayon pool: {e}"));
                rayon_sum(input, input_variant.work, &pool)
            }
            Method::Con1 => con1_sum(input, input_variant.work, input_variant.num_threads, 1),
            Method::Con1Chunk => con1_sum(
                input,
                input_variant.work,
                input_variant.num_threads,
                CHUNK_SIZE,
            ),
            Method::RecIter => concurrent_recursive_iter_sum(
                input,
                input_variant.work,
                input_variant.num_threads,
                1,
            ),
            Method::RecIterChunk => concurrent_recursive_iter_sum(
                input,
                input_variant.work,
                input_variant.num_threads,
                CHUNK_SIZE,
            ),
        }
    }

    fn validate_output(
        &self,
        input_variant: &Self::InputFactors,
        input: &Self::Input,
        output: &Self::Output,
    ) {
        let expected = seq_sum(input, input_variant.work);
        assert_eq!(expected, *output);
    }
}

fn run(c: &mut Criterion) {
    let treatments: Vec<_> = THREADS
        .iter()
        .map(|&num_threads| Input {
            num_threads,
            nodes: 40_000,
            roots: 100,
            max_children: 8,
            work: 300,
            seed: 42,
        })
        .collect();

    let variants = vec![
        Method::Seq,
        Method::Rayon,
        Method::Con1,
        Method::Con1Chunk,
        Method::RecIter,
        Method::RecIterChunk,
    ];

    Exp.bench(c, "ben", &treatments, &variants);
}

criterion_group!(benches, run);
criterion_main!(benches);
