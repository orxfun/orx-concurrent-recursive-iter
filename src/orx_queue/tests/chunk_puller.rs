use crate::orx_queue::{chunk_puller::DynChunkPuller, queue::Queue};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use orx_concurrent_iter::ChunkPuller;
use orx_concurrent_queue::{ConcurrentQueue, DefaultConPinnedVec};

fn vec_strings(n: usize) -> Vec<String> {
    (0..n).map(|i| i.to_string()).collect()
}

#[allow(clippy::ptr_arg)]
fn extend_fn(s: &String, queue: &Queue<String, DefaultConPinnedVec<String>>) {
    let num: usize = s.parse().unwrap_or(0);
    for i in 0..num {
        queue.push(i.to_string());
    }
}

#[test]
fn resize_for_chunk_size_initial_state() {
    let concurrent_queue: ConcurrentQueue<String, DefaultConPinnedVec<String>> =
        ConcurrentQueue::new();

    for s in vec_strings(5) {
        concurrent_queue.push(s);
    }

    let puller = DynChunkPuller::new(&extend_fn, &concurrent_queue, 8);

    assert_eq!(puller.chunk_size(), 8);
}

#[test]
fn resize_for_chunk_size_updates_chunk_size() {
    let concurrent_queue: ConcurrentQueue<String, DefaultConPinnedVec<String>> =
        ConcurrentQueue::new();

    for s in vec_strings(5) {
        concurrent_queue.push(s);
    }

    let mut puller = DynChunkPuller::new(&extend_fn, &concurrent_queue, 8);

    assert_eq!(puller.chunk_size(), 8);

    puller.resize_for_chunk_size(16);

    assert_eq!(puller.chunk_size(), 16);
}

#[test]
fn resize_for_chunk_size_multiple_resizes() {
    let concurrent_queue: ConcurrentQueue<String, DefaultConPinnedVec<String>> =
        ConcurrentQueue::new();

    for s in vec_strings(5) {
        concurrent_queue.push(s);
    }

    let mut puller = DynChunkPuller::new(&extend_fn, &concurrent_queue, 8);

    assert_eq!(puller.chunk_size(), 8);

    puller.resize_for_chunk_size(16);
    assert_eq!(puller.chunk_size(), 16);

    puller.resize_for_chunk_size(32);
    assert_eq!(puller.chunk_size(), 32);

    puller.resize_for_chunk_size(64);
    assert_eq!(puller.chunk_size(), 64);
}

#[test]
fn resize_for_chunk_size_downsize() {
    let concurrent_queue: ConcurrentQueue<String, DefaultConPinnedVec<String>> =
        ConcurrentQueue::new();

    for s in vec_strings(5) {
        concurrent_queue.push(s);
    }

    let mut puller = DynChunkPuller::new(&extend_fn, &concurrent_queue, 64);

    assert_eq!(puller.chunk_size(), 64);

    puller.resize_for_chunk_size(16);

    assert_eq!(puller.chunk_size(), 16);
}

#[test]
fn resize_for_chunk_size_zero_size() {
    let concurrent_queue: ConcurrentQueue<String, DefaultConPinnedVec<String>> =
        ConcurrentQueue::new();

    for s in vec_strings(5) {
        concurrent_queue.push(s);
    }

    let mut puller = DynChunkPuller::new(&extend_fn, &concurrent_queue, 10);

    puller.resize_for_chunk_size(0);

    assert_eq!(puller.chunk_size(), 0);
}

#[test]
fn resize_for_chunk_size_large_size() {
    let concurrent_queue: ConcurrentQueue<String, DefaultConPinnedVec<String>> =
        ConcurrentQueue::new();

    for s in vec_strings(5) {
        concurrent_queue.push(s);
    }

    let mut puller = DynChunkPuller::new(&extend_fn, &concurrent_queue, 1024);

    puller.resize_for_chunk_size(1_000_000);

    assert_eq!(puller.chunk_size(), 1_000_000);
}

#[test]
fn resize_for_chunk_size_one_size() {
    let concurrent_queue: ConcurrentQueue<String, DefaultConPinnedVec<String>> =
        ConcurrentQueue::new();

    for s in vec_strings(5) {
        concurrent_queue.push(s);
    }

    let mut puller = DynChunkPuller::new(&extend_fn, &concurrent_queue, 1024);

    puller.resize_for_chunk_size(1);

    assert_eq!(puller.chunk_size(), 1);
}

#[test]
fn resize_for_chunk_size_same_size() {
    let concurrent_queue: ConcurrentQueue<String, DefaultConPinnedVec<String>> =
        ConcurrentQueue::new();

    for s in vec_strings(5) {
        concurrent_queue.push(s);
    }

    let mut puller = DynChunkPuller::new(&extend_fn, &concurrent_queue, 32);

    assert_eq!(puller.chunk_size(), 32);

    puller.resize_for_chunk_size(32);

    assert_eq!(puller.chunk_size(), 32);
}

#[test]
fn resize_for_chunk_size_rapid_changes() {
    let concurrent_queue: ConcurrentQueue<String, DefaultConPinnedVec<String>> =
        ConcurrentQueue::new();

    for s in vec_strings(5) {
        concurrent_queue.push(s);
    }

    let mut puller = DynChunkPuller::new(&extend_fn, &concurrent_queue, 8);

    for new_size in &[16, 8, 32, 1, 64, 2] {
        puller.resize_for_chunk_size(*new_size);
        assert_eq!(puller.chunk_size(), *new_size);
    }
}
