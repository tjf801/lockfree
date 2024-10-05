use std::{ptr::NonNull, sync::atomic::AtomicUsize};
use std::marker::PhantomData;
use std::cell::UnsafeCell;

// https://www.stroustrup.com/lock-free-vector.pdf

struct ConcurrentVec<T> {
    ptr: NonNull<UnsafeCell<[T]>>,
    descriptor: ConcurrentVecDescriptor<T>
}

struct ConcurrentVecDescriptor<T> {
    size: AtomicUsize,
    counter: AtomicUsize,
    write_descriptor: Option<()>,
    _a: PhantomData<T> // todo
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_new_empty() {
        let x = Vec::<i32>::new();
        
    }
}
