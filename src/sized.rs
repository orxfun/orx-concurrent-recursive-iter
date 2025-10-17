trait Seal {}

/// Marker trait determining whether the iterator is exact-sized or not.
#[allow(private_bounds)]
pub trait Size: Seal + Send + Sync {}

/// The iterator has known exact size.
pub struct ExactSize(pub(super) usize);
impl Seal for ExactSize {}
impl Size for ExactSize {}

/// Size of the iterator is unknown ahead of time.
pub struct UnknownSize;
impl Seal for UnknownSize {}
impl Size for UnknownSize {}
