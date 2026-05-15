// cargo run --release --example recursive_iter_tune

use orx_concurrent_iter::ConcurrentIter;
use orx_concurrent_recursive_iter::{
    ConcurrentIterCross, ConcurrentRecursiveIter, ConcurrentRecursiveIterShards, Queue,
};
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use rayon::{Scope, ThreadPool, ThreadPoolBuilder, scope};
use std::num::NonZeroUsize;
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

#[derive(Clone, Copy, Debug)]
enum Method {
    Seq,
    Rayon,
    RecIter,
    RecIterShards,
    CrossbeamDeque,
    All,
}

impl core::str::FromStr for Method {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "seq" => Ok(Self::Seq),
            "rayon" => Ok(Self::Rayon),
            "reciter" => Ok(Self::RecIter),
            "reciter-shards" => Ok(Self::RecIterShards),
            "crossbeam" => Ok(Self::CrossbeamDeque),
            "all" => Ok(Self::All),
            _ => Err(format!(
                "unknown method: {s}; expected one of seq|rayon|reciter|reciter-shards|crossbeam|all"
            )),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Args {
    nodes: usize,
    roots: usize,
    max_children: usize,
    work: usize,
    seed: u64,
    method: Method,
    repetitions: usize,
    warmup: usize,
    num_threads: usize,
    num_shards: usize,
}

impl Default for Args {
    fn default() -> Self {
        let num_threads = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1);

        Self {
            nodes: 40_000,
            roots: 50,
            max_children: 8,
            work: 250,
            seed: 42,
            method: Method::All,
            repetitions: 5,
            warmup: 1,
            num_threads,
            num_shards: num_threads,
        }
    }
}

impl Args {
    fn from_env() -> Self {
        let mut args = Self::default();

        let mut iter = std::env::args().skip(1);
        while let Some(flag) = iter.next() {
            let value = iter
                .next()
                .unwrap_or_else(|| panic!("missing value after {flag}"));

            match flag.as_str() {
                "--nodes" => args.nodes = parse_usize("nodes", &value),
                "--roots" => args.roots = parse_usize("roots", &value),
                "--max-children" => args.max_children = parse_usize("max-children", &value),
                "--work" => args.work = parse_usize("work", &value),
                "--seed" => args.seed = parse_u64("seed", &value),
                "--method" => args.method = value.parse().unwrap_or_else(|e: String| panic!("{e}")),
                "--repetitions" => args.repetitions = parse_usize("repetitions", &value),
                "--warmup" => args.warmup = parse_usize("warmup", &value),
                "--num-threads" => args.num_threads = parse_usize("num-threads", &value),
                "--num-shards" => args.num_shards = parse_usize("num-shards", &value),
                _ => panic!("unknown argument: {flag}"),
            }
        }

        args
    }
}

fn parse_usize(name: &str, value: &str) -> usize {
    value
        .parse::<usize>()
        .unwrap_or_else(|_| panic!("invalid --{name}: {value}"))
}

fn parse_u64(name: &str, value: &str) -> u64 {
    value
        .parse::<u64>()
        .unwrap_or_else(|_| panic!("invalid --{name}: {value}"))
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
        .unwrap_or_else(|| panic!("--num-shards must be greater than zero"));

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

fn run_one(name: &str, reps: usize, mut f: impl FnMut() -> u64) -> (u64, f64, f64, f64) {
    let mut times_ms = Vec::with_capacity(reps);
    let mut last_sum = 0u64;

    for _ in 0..reps {
        let start = Instant::now();
        last_sum = f();
        let elapsed_ms = start.elapsed().as_secs_f64() * 1_000.0;
        times_ms.push(elapsed_ms);
    }

    let avg = times_ms.iter().sum::<f64>() / times_ms.len() as f64;
    let min = times_ms.iter().copied().fold(f64::INFINITY, f64::min);
    let max = times_ms.iter().copied().fold(0.0, f64::max);

    println!(
        "{name:<16} | avg={avg:>8.3} ms | min={min:>8.3} ms | max={max:>8.3} ms | sum={last_sum}"
    );

    (last_sum, avg, min, max)
}

fn main() {
    let args = Args::from_env();

    println!("\nRecursive iterator tuning workload");
    println!(
        "nodes={} roots={} max_children={} work={} seed={} reps={} warmup={} method={:?} num_threads={} num_shards={}",
        args.nodes,
        args.roots,
        args.max_children,
        args.work,
        args.seed,
        args.repetitions,
        args.warmup,
        args.method,
        args.num_threads,
        args.num_shards
    );

    let fs = FileSystem::generate(args.nodes, args.roots, args.max_children, args.seed);

    let baseline = seq_sum(&fs, args.work);
    println!("baseline seq sum = {baseline}");

    let selected: &[Method] = match args.method {
        Method::All => &[
            Method::Seq,
            Method::Rayon,
            Method::RecIter,
            Method::RecIterShards,
            Method::CrossbeamDeque,
        ],
        Method::Seq => &[Method::Seq],
        Method::Rayon => &[Method::Rayon],
        Method::RecIter => &[Method::RecIter],
        Method::RecIterShards => &[Method::RecIterShards],
        Method::CrossbeamDeque => &[Method::CrossbeamDeque],
    };

    let rayon_pool = selected
        .iter()
        .any(|m| matches!(m, Method::Rayon))
        .then(|| {
            ThreadPoolBuilder::new()
                .num_threads(args.num_threads.max(1))
                .build()
                .unwrap_or_else(|e| panic!("failed to build rayon thread pool: {e}"))
        });

    for method in selected {
        for _ in 0..args.warmup {
            let _ = match method {
                Method::Seq => seq_sum(&fs, args.work),
                Method::Rayon => rayon_sum(
                    &fs,
                    args.work,
                    rayon_pool
                        .as_ref()
                        .unwrap_or_else(|| panic!("rayon thread pool must exist for rayon method")),
                ),
                Method::RecIter => recursive_iter_sum(&fs, args.work, args.num_threads),
                Method::RecIterShards => {
                    recursive_iter_shards_sum(&fs, args.work, args.num_threads, args.num_shards)
                }
                Method::CrossbeamDeque => {
                    crossbeam_iter_cross_sum(&fs, args.work, args.num_threads)
                }
                Method::All => unreachable!(),
            };
        }
    }

    println!("\nMeasured runs:");

    let mut reference: Option<u64> = None;
    for method in selected {
        let (sum, _, _, _) = match method {
            Method::Seq => run_one("sequential", args.repetitions, || seq_sum(&fs, args.work)),
            Method::Rayon => run_one("rayon", args.repetitions, || {
                rayon_sum(
                    &fs,
                    args.work,
                    rayon_pool
                        .as_ref()
                        .unwrap_or_else(|| panic!("rayon thread pool must exist for rayon method")),
                )
            }),
            Method::RecIter => run_one("recursive-iter", args.repetitions, || {
                recursive_iter_sum(&fs, args.work, args.num_threads)
            }),
            Method::RecIterShards => run_one("reciter-shards", args.repetitions, || {
                recursive_iter_shards_sum(&fs, args.work, args.num_threads, args.num_shards)
            }),
            Method::CrossbeamDeque => run_one("crossbeam-deque", args.repetitions, || {
                crossbeam_iter_cross_sum(&fs, args.work, args.num_threads)
            }),
            Method::All => unreachable!(),
        };

        if let Some(expected) = reference {
            assert_eq!(sum, expected, "sum mismatch for method {method:?}");
        } else {
            reference = Some(sum);
        }
    }

    println!("\nAll selected methods produced identical sums.");
    println!("\nArgs:");
    println!("  --nodes <usize>");
    println!("  --roots <usize>");
    println!("  --max-children <usize>");
    println!("  --work <usize>");
    println!("  --seed <u64>");
    println!("  --method <seq|rayon|reciter|reciter-shards|crossbeam|all>");
    println!("  --repetitions <usize>");
    println!("  --warmup <usize>");
    println!("  --num-threads <usize>");
    println!("  --num-shards <usize>");
}
