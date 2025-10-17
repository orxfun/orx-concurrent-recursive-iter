trait Seal {}

pub trait Size: Seal {}

pub struct ExactSized;
impl Seal for ExactSized {}
impl Size for ExactSized {}

pub struct UnknownSized;
impl Seal for UnknownSized {}
impl Size for UnknownSized {}
