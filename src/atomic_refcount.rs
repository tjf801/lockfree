use std::{cell::UnsafeCell, marker::PhantomData};
use std::ptr::NonNull;
use std::sync::atomic;
use std::mem::ManuallyDrop;

use atomic::{AtomicUsize, Ordering};

pub struct Arc<T: ?Sized> {
    ptr: NonNull<ArcInner<T>>,
    phantom: PhantomData<ArcInner<T>>,
}

// SAFETY: since `T` is dropped by whatever thread is the last `Arc`, `Arc<T>: Send + Sync` if `T: Send`.
//         since `Arc`'s entire point is to provide an `&T` across threads, `Arc<T>: Send + Sync` if `T: Sync`.
unsafe impl<T: ?Sized + Sync + Send> Send for Arc<T> {}
unsafe impl<T: ?Sized + Sync + Send> Sync for Arc<T> {}

pub struct WeakArc<T: ?Sized> {
    ptr: NonNull<ArcInner<T>>
}

// SAFETY: see comment for `Arc<T>`
unsafe impl<T: ?Sized + Sync + Send> Send for WeakArc<T> {}
unsafe impl<T: ?Sized + Sync + Send> Sync for WeakArc<T> {}

struct ArcInner<T: ?Sized> {
    strong_count: AtomicUsize,
    weak_count: AtomicUsize,
    data: UnsafeCell<ManuallyDrop<T>>,
}


impl<T> Arc<T> {
    pub fn new(data: T) -> Self {
        Self {
            ptr: NonNull::new(Box::into_raw(Box::new(ArcInner {
                strong_count: AtomicUsize::new(1),
                weak_count: AtomicUsize::new(1),
                data: UnsafeCell::new(ManuallyDrop::new(data))
            }))).expect("Box<T> guaruntees that into_raw() is non-null"),
            phantom: PhantomData
        }
    }
}

impl<T: ?Sized> Arc<T> {
    fn inner(&self) -> &ArcInner<T> {
        // SAFETY: Pointer is valid, and no exclusive references exist
        unsafe { self.ptr.as_ref() }
    }
    
    pub fn get_mut(arc: &mut Self) -> Option<&mut T> {
        if arc.inner().weak_count.compare_exchange(1, usize::MAX, Ordering::Acquire, Ordering::Relaxed).is_err() {
            return None
        }
        
        let is_unique = arc.inner().strong_count.load(Ordering::Relaxed) == 1;
        
        arc.inner().weak_count.store(1, Ordering::Relaxed);
        if !is_unique {
            return None
        }
        
        atomic::fence(Ordering::Acquire);
        unsafe { Some(&mut *arc.inner().data.get()) }
    }
    
    pub fn downgrade(arc: Self) -> WeakArc<T> {
        todo!()
    }
}

impl<T: ?Sized> std::ops::Deref for Arc<T> {
    type Target = T;
    
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.inner().data.get() }
    }
}

impl<T: ?Sized> Clone for Arc<T> {
    fn clone(&self) -> Self {
        let old_size = self.inner().strong_count.fetch_add(1, Ordering::Relaxed);
        
        if old_size >= isize::MAX as usize {
            panic!("too many references to Arc") // TODO: do something more than just panicking..?
        }
        
        Self {
            ptr: self.ptr,
            phantom: PhantomData
        }
    }
}

impl<T: ?Sized> Drop for Arc<T> {
    fn drop(&mut self) {
        // Ordering::Release guarantees that any previous increments are visible
        if self.inner().strong_count.fetch_sub(1, Ordering::Release) == 1 {
            atomic::fence(Ordering::Acquire);
            
            // SAFETY: since the refcnt is now 0, nothing else is referencing the data.
            unsafe {
                ManuallyDrop::drop(&mut *self.inner().data.get())
            }
            
            // Since there are no `Arc<T>`s left, we drop the weak reference collectively held by all of the strong references.
            drop(WeakArc { ptr: self.ptr })
        }
    }
}


impl<T: ?Sized> WeakArc<T> {
    fn inner(&self) -> &ArcInner<T> {
        unsafe { self.ptr.as_ref() }
    }
    
    // N.B: this function can lock.
    pub fn upgrade(&self) -> Option<Arc<T>> {
        let mut n = self.inner().strong_count.load(Ordering::Relaxed);
        loop {
            if n == 0 { return None }
            assert!(n < isize::MAX as usize);
            if let Err(e) = self.inner().strong_count
                .compare_exchange_weak(n, n+1, Ordering::Relaxed, Ordering::Relaxed) {
                n = e;
                continue
            }
            return Some(Arc { ptr: self.ptr, phantom: PhantomData })
        }
    }
}

impl<T: ?Sized> Clone for WeakArc<T> {
    fn clone(&self) -> Self {
        let old_count = self.inner().weak_count.fetch_add(1, Ordering::Relaxed);
        
        if old_count >= isize::MAX as usize {
            std::process::abort()
        }
        
        Self { ptr: self.ptr }
    }
}

impl<T: ?Sized> Drop for WeakArc<T> {
    fn drop(&mut self) {
        if self.inner().weak_count.fetch_sub(1, Ordering::Release) == 1 {
            atomic::fence(Ordering::Acquire);
            
            drop(
                unsafe { Box::from_raw(self.ptr.as_ptr()) }
            )
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_basic() {
        static NUM_DROPS: AtomicUsize = AtomicUsize::new(0);
        struct DropDetector;
        impl Drop for DropDetector {
            fn drop(&mut self) {
                NUM_DROPS.fetch_add(1, Ordering::Relaxed);
            }
        }
        
        let x = Arc::new(("Hello world", DropDetector));
        let y = x.clone();
        
        let t = std::thread::spawn(move || {
            assert_eq!(x.0, "Hello world");
        });
        
        assert_eq!(y.0, "Hello world");
        
        t.join().unwrap();
        
        assert_eq!(NUM_DROPS.load(Ordering::Relaxed), 0);
        
        drop(y);
        
        assert_eq!(NUM_DROPS.load(Ordering::Relaxed), 1);
    }
}
