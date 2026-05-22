use crossbeam_deque::Injector;

pub(super) type Queue<T> = Injector<T>;