trait Seal {}

/// Marker trait determining whether the iterator is exact-sized or not.
#[allow(private_bounds)]
pub trait Size: Seal + Send + Sync {
    /// Type of the exact length.
    type ExactLen;
}

/// The iterator has known exact size.
pub struct ExactSize;
impl Seal for ExactSize {}
impl Size for ExactSize {
    type ExactLen = usize;
}

/// Size of the iterator is unknown ahead of time.
pub struct UnknownSize;
impl Seal for UnknownSize {}
impl Size for UnknownSize {
    type ExactLen = ();
}
