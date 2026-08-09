use crate::cross_no_std::{
    chunk_puller::DynChunkPuller, con_iter::ConcurrentRecursiveIterCrossbeamNoStd,
};
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use orx_concurrent_iter::ChunkPuller;

fn vec_strings(n: usize) -> Vec<String> {
    (0..n).map(|i| i.to_string()).collect()
}

fn extend_fn(s: &String) -> Vec<String> {
    let num: usize = s.parse().unwrap_or(0);
    (0..num).map(|i| i.to_string()).collect()
}

#[test]
fn resize_for_chunk_size_initial_state() {
    let iter = ConcurrentRecursiveIterCrossbeamNoStd::new(vec_strings(5), extend_fn, None, Some(1));

    let puller = DynChunkPuller {
        iter: &iter,
        chunk_size: 8,
        thread_idx: None,
        chunk_buffer: Vec::new(),
    };

    assert_eq!(puller.chunk_size(), 8);
}

#[test]
fn resize_for_chunk_size_updates_chunk_size() {
    let iter = ConcurrentRecursiveIterCrossbeamNoStd::new(vec_strings(5), extend_fn, None, Some(1));

    let mut puller = DynChunkPuller {
        iter: &iter,
        chunk_size: 8,
        thread_idx: None,
        chunk_buffer: Vec::new(),
    };

    assert_eq!(puller.chunk_size(), 8);

    puller.resize_for_chunk_size(16);

    assert_eq!(puller.chunk_size(), 16);
}

#[test]
fn resize_for_chunk_size_allocates_capacity() {
    let iter = ConcurrentRecursiveIterCrossbeamNoStd::new(vec_strings(5), extend_fn, None, Some(1));

    let mut puller = DynChunkPuller {
        iter: &iter,
        chunk_size: 10,
        thread_idx: None,
        chunk_buffer: Vec::new(),
    };

    let initial_capacity = puller.chunk_buffer.capacity();

    puller.resize_for_chunk_size(32);

    assert!(puller.chunk_buffer.capacity() >= 32);
    assert!(puller.chunk_buffer.capacity() >= initial_capacity);
}

#[test]
fn resize_for_chunk_size_with_existing_elements() {
    let iter = ConcurrentRecursiveIterCrossbeamNoStd::new(vec_strings(5), extend_fn, None, Some(1));

    let mut puller = DynChunkPuller {
        iter: &iter,
        chunk_size: 10,
        thread_idx: None,
        chunk_buffer: vec!["a".to_string(), "b".to_string(), "c".to_string()],
    };

    let element_count = puller.chunk_buffer.len();
    assert_eq!(element_count, 3);

    puller.resize_for_chunk_size(20);

    assert_eq!(puller.chunk_size(), 20);
    // Elements should not be affected
    assert_eq!(puller.chunk_buffer.len(), 3);
    // Capacity should be sufficient for the new chunk size
    assert!(puller.chunk_buffer.capacity() >= 20);
}

#[test]
fn resize_for_chunk_size_multiple_resizes() {
    let iter = ConcurrentRecursiveIterCrossbeamNoStd::new(vec_strings(5), extend_fn, None, Some(1));

    let mut puller = DynChunkPuller {
        iter: &iter,
        chunk_size: 8,
        thread_idx: None,
        chunk_buffer: Vec::new(),
    };

    assert_eq!(puller.chunk_size(), 8);

    puller.resize_for_chunk_size(16);
    assert_eq!(puller.chunk_size(), 16);
    assert!(puller.chunk_buffer.capacity() >= 16);

    puller.resize_for_chunk_size(32);
    assert_eq!(puller.chunk_size(), 32);
    assert!(puller.chunk_buffer.capacity() >= 32);

    puller.resize_for_chunk_size(64);
    assert_eq!(puller.chunk_size(), 64);
    assert!(puller.chunk_buffer.capacity() >= 64);
}

#[test]
fn resize_for_chunk_size_downsize() {
    let iter = ConcurrentRecursiveIterCrossbeamNoStd::new(vec_strings(5), extend_fn, None, Some(1));

    let mut puller = DynChunkPuller {
        iter: &iter,
        chunk_size: 64,
        thread_idx: None,
        chunk_buffer: Vec::with_capacity(64),
    };

    assert_eq!(puller.chunk_size(), 64);

    puller.resize_for_chunk_size(16);

    assert_eq!(puller.chunk_size(), 16);
    // When downsizing, saturating_sub should handle it gracefully
    // and reserve_additional should not allocate (or minimal allocation)
}

#[test]
fn resize_for_chunk_size_saturating_sub_behavior() {
    let iter = ConcurrentRecursiveIterCrossbeamNoStd::new(vec_strings(5), extend_fn, None, Some(1));

    let mut puller = DynChunkPuller {
        iter: &iter,
        chunk_size: 10,
        thread_idx: Some(0),
        chunk_buffer: vec!["x".to_string(); 100],
    };

    assert_eq!(puller.chunk_buffer.len(), 100);

    // Resize to 50 when buffer already has 100 elements
    // saturating_sub(50 - 100) should return 0, so no additional reserve
    puller.resize_for_chunk_size(50);

    assert_eq!(puller.chunk_size(), 50);
    // Buffer size is unchanged
    assert_eq!(puller.chunk_buffer.len(), 100);
    // Capacity should remain at least 100
    assert!(puller.chunk_buffer.capacity() >= 100);
}

#[test]
fn resize_for_chunk_size_zero_size() {
    let iter = ConcurrentRecursiveIterCrossbeamNoStd::new(vec_strings(5), extend_fn, None, Some(1));

    let mut puller = DynChunkPuller {
        iter: &iter,
        chunk_size: 10,
        thread_idx: None,
        chunk_buffer: Vec::new(),
    };

    puller.resize_for_chunk_size(0);

    assert_eq!(puller.chunk_size(), 0);
}

#[test]
fn resize_for_chunk_size_large_size() {
    let iter = ConcurrentRecursiveIterCrossbeamNoStd::new(vec_strings(5), extend_fn, None, Some(1));

    let mut puller = DynChunkPuller {
        iter: &iter,
        chunk_size: 1024,
        thread_idx: None,
        chunk_buffer: Vec::new(),
    };

    puller.resize_for_chunk_size(1_000_000);

    assert_eq!(puller.chunk_size(), 1_000_000);
    assert!(puller.chunk_buffer.capacity() >= 1_000_000);
}
