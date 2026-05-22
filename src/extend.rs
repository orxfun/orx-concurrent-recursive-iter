pub fn iter_len<I>(iter: &I) -> usize
where
    I: Iterator,
{
    match iter.size_hint() {
        // Exact size iterator
        (lb, Some(ub)) if lb == ub => lb,
        _ => todo!(),
    }
}
