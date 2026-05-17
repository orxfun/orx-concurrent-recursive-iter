use criterion::{Criterion, criterion_group, criterion_main};
use enum_iterator::{Sequence, all};
use orx_concurrent_iter::{ChunkPuller, ConcurrentIter};
use orx_concurrent_recursive_iter::{
    ConcurrentIterCross, ConcurrentIterCrossSeg, ConcurrentRecursiveIter,
    ConcurrentRecursiveIterShards, ConcurrentRecursiveIterShards2, Queue,
};
use orx_criterion::{Experiment, Factors};
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use rayon::{Scope, ThreadPool, ThreadPoolBuilder, scope};
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

fn recursive_iter_shards_sum_chunk64(
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
                let mut puller = iter.chunk_puller(64);
                while let Some(chunk) = puller.pull() {
                    local_sum += chunk
                        .into_iter()
                        .map(|idx| fs.nodes[idx].compute_score(work))
                        .sum::<u64>();
                }
                local_sum
            }));
        }

        handles.into_iter().map(|h| h.join().unwrap()).sum()
    })
}

fn recursive_iter_shards2_sum(
    fs: &FileSystem,
    work: usize,
    num_threads: usize,
    num_shards: usize,
) -> u64 {
    let iter = ConcurrentRecursiveIterShards2::new_exact_with_shards(
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

fn crossbeam_iter_cross_sum(fs: &FileSystem, work: usize, num_threads: usize) -> u64 {
    let iter = ConcurrentIterCross::new_exact(
        fs.roots.iter().copied(),
        |idx: &usize| fs.nodes[*idx].children.clone(),
        fs.nodes.len(),
    );

    iter.run_with_threads(num_threads, |idx: &usize| {
        fs.nodes[*idx].compute_score(work)
    })
}

fn crossbeam_iter_cross_sum2(fs: &FileSystem, work: usize, num_threads: usize) -> u64 {
    let iter = ConcurrentIterCross::new_exact(
        fs.roots.iter().copied(),
        |idx: &usize| fs.nodes[*idx].children.clone(),
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

fn crossbeam_iter_cross_sum2_chunk64(fs: &FileSystem, work: usize, num_threads: usize) -> u64 {
    let iter = ConcurrentIterCross::new_exact(
        fs.roots.iter().copied(),
        |idx: &usize| fs.nodes[*idx].children.clone(),
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
                let mut puller = iter.chunk_puller(64);
                while let Some(chunk) = puller.pull() {
                    local_sum += chunk
                        .into_iter()
                        .map(|idx| fs.nodes[idx].compute_score(work))
                        .sum::<u64>();
                }
                local_sum
            }));
        }

        handles.into_iter().map(|h| h.join().unwrap()).sum()
    })
}

fn crossbeam_iter_cross_seg_sum(fs: &FileSystem, work: usize, num_threads: usize) -> u64 {
    let iter = ConcurrentIterCrossSeg::new_exact(
        fs.roots.iter().copied(),
        |idx: &usize| fs.nodes[*idx].children.clone(),
        fs.nodes.len(),
    );

    iter.run_with_threads(num_threads, |idx: &usize| {
        fs.nodes[*idx].compute_score(work)
    })
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

    fn factor_names_short() -> Vec<&'static str> {
        vec!["nt", "n", "r", "w"]
    }
}

#[derive(Debug, Sequence)]
enum Method {
    Seq,
    Rayon,
    RecIter,
    RecIterShards1,
    RecIterShards8,
    RecIterShards1Chunk64,
    RecIterShards8Chunk64,
    RecIterShards2_1,
    RecIterShards2_2,
    CrossbeamDeque,
    CrossbeamDeque2,
    CrossbeamDeque2Chunk64,
    CrossbeamSegQueue,
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
                Self::RecIter => "orx",
                Self::RecIterShards1 => "orx-s1",
                Self::RecIterShards8 => "orx-s8",
                Self::RecIterShards1Chunk64 => "orx-s1-c64",
                Self::RecIterShards8Chunk64 => "orx-s8-c64",
                Self::RecIterShards2_1 => "orx2-s1",
                Self::RecIterShards2_2 => "orx2-s2",
                Self::CrossbeamDeque => "cb",
                Self::CrossbeamDeque2 => "cb2",
                Self::CrossbeamDeque2Chunk64 => "cb2-c64",
                Self::CrossbeamSegQueue => "cbq",
            }
            .to_string(),
        ]
    }

    fn factor_levels_short(&self) -> Vec<String> {
        vec![
            match self {
                Self::Seq => "s",
                Self::Rayon => "r",
                Self::RecIter => "o",
                Self::RecIterShards1 => "o-s1",
                Self::RecIterShards8 => "o-s8",
                Self::RecIterShards1Chunk64 => "o-s1-c64",
                Self::RecIterShards8Chunk64 => "o-s8-c64",
                Self::RecIterShards2_1 => "o2-s1",
                Self::RecIterShards2_2 => "o2-s2",
                Self::CrossbeamDeque => "cb",
                Self::CrossbeamDeque2 => "cb2",
                Self::CrossbeamDeque2Chunk64 => "cb2-c64",
                Self::CrossbeamSegQueue => "cbq",
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
            Method::RecIter => {
                recursive_iter_sum(input, input_variant.work, input_variant.num_threads)
            }
            Method::RecIterShards1 => recursive_iter_shards_sum(
                input,
                input_variant.work,
                input_variant.num_threads,
                input_variant.num_threads,
            ),
            Method::RecIterShards8 => {
                let num_shards = (input_variant.num_threads / 8).max(1);
                recursive_iter_shards_sum(
                    input,
                    input_variant.work,
                    input_variant.num_threads,
                    num_shards,
                )
            }
            Method::RecIterShards1Chunk64 => recursive_iter_shards_sum_chunk64(
                input,
                input_variant.work,
                input_variant.num_threads,
                input_variant.num_threads,
            ),
            Method::RecIterShards8Chunk64 => {
                let num_shards = (input_variant.num_threads / 8).max(1);
                recursive_iter_shards_sum_chunk64(
                    input,
                    input_variant.work,
                    input_variant.num_threads,
                    num_shards,
                )
            }
            Method::RecIterShards2_1 => {
                recursive_iter_shards2_sum(input, input_variant.work, input_variant.num_threads, 1)
            }
            Method::RecIterShards2_2 => {
                recursive_iter_shards2_sum(input, input_variant.work, input_variant.num_threads, 2)
            }
            Method::CrossbeamDeque => {
                crossbeam_iter_cross_sum(input, input_variant.work, input_variant.num_threads)
            }
            Method::CrossbeamDeque2 => {
                crossbeam_iter_cross_sum2(input, input_variant.work, input_variant.num_threads)
            }
            Method::CrossbeamDeque2Chunk64 => crossbeam_iter_cross_sum2_chunk64(
                input,
                input_variant.work,
                input_variant.num_threads,
            ),
            Method::CrossbeamSegQueue => {
                crossbeam_iter_cross_seg_sum(input, input_variant.work, input_variant.num_threads)
            }
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

    let variants: Vec<_> = all::<Method>().collect();
    let variants = vec![
        Method::Rayon,
        Method::CrossbeamDeque,
        Method::CrossbeamDeque2,
        Method::CrossbeamDeque2Chunk64,
        Method::CrossbeamSegQueue,
    ];

    Exp.bench(c, "recursive_iter_scaling", &treatments, &variants);
}

criterion_group!(benches, run);
criterion_main!(benches);
