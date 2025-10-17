trait Seal {}

/// Marker trait determining whether the iterator is exact-sized or not.
#[allow(private_bounds)]
pub trait Size: Seal {}

/// The iterator has known exact size.
pub struct ExactSized;
impl Seal for ExactSized {}
impl Size for ExactSized {}

/// Size of the iterator is unknown ahead of time.
pub struct UnknownSized;
impl Seal for UnknownSized {}
impl Size for UnknownSized {}
