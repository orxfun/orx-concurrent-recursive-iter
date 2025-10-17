trait Seal {}

/// Marker trait determining whether the iterator is exact-sized or not.
#[allow(private_bounds)]
pub trait Size: Seal + Send + Sync + Clone + Copy {
    /// Returns the total number of elements that the recursive iterator
    /// will generate if completely consumed.
    ///
    /// Returns None if this is not known ahead of time.
    fn len(self) -> Option<usize>;
}

/// The iterator has known exact size.
#[derive(Clone, Copy)]
pub struct ExactSize(pub(super) usize);
impl Seal for ExactSize {}
impl Size for ExactSize {
    #[inline(always)]
    fn len(self) -> Option<usize> {
        Some(self.0)
    }
}

/// Size of the iterator is unknown ahead of time.
#[derive(Clone, Copy)]
pub struct UnknownSize;
impl Seal for UnknownSize {}
impl Size for UnknownSize {
    #[inline(always)]
    fn len(self) -> Option<usize> {
        None
    }
}
