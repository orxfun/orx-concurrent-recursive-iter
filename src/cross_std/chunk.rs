use alloc::vec::Drain;

pub type DynChunk<'a, T> = Drain<'a, T>;
