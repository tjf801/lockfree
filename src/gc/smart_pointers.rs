//! Smart pointer types for Garbage Collected (GCed) memory.
//! 
//! This is the main API for interacting with the garbage collector.
//! 
//! TODO: consider a `GcPin` pointer?

use std::alloc::{Allocator, Layout};
use std::marker::{PhantomData, Unsize};
use std::mem::{self, MaybeUninit};
use std::ops::{CoerceUnsized, Deref, DerefPure, DispatchFromDyn};
use std::ptr::{NonNull, Unique};

use super::allocator::{GCAllocatorError, GC_ALLOCATOR};


/// Shared access to Garbage Collected (GCed) memory.
/// 
/// A smart pointer to data that is owned by the garbage collector. This type is similar to an [`Arc`], in
/// that it is basically just a pointer to some data, but differs in the fact that the bookkeeping necessary
/// for knowing when to free the data is done seperately. This allows the type to not only be `Clone`, but
/// since the GC will scan the stack and heap for pointers, it means that the entire type does not have
/// to do any incrementing or decrementing of a reference count, making it [`Copy`]. As such, passing around
/// something like a `Gc<`[`Mutex`]`<T>>` will have nearly zero overhead compared to simply making it a
/// `static` variable (ignoring the performance hit from running the GC to begin with), and will be
/// significantly faster than [`Arc`]s with large numbers of [`clone`] calls.
/// 
/// It should be noted that `T` must be [`Send`] to use this type, since the value is going to have its
/// ownership sent to the garbage collector's thread as it is being freed. Similarly, since there are no
/// borrow checker semantics on `Gc`, the type must live for an arbitrarily long time (i.e: have a `'static`
/// bound), because, as an example, if you put a temporary reference into GCed memory, you could potentially
/// use it for arbitrarily long, even after its lifetime had ended.
/// 
/// [`Arc`]: std::sync::Arc
/// [`Mutex`]: std::sync::Mutex
/// [`clone`]: Clone::clone
#[repr(transparent)]
pub struct Gc<T: ?Sized + Send + 'static>(NonNull<T>, PhantomData<&'static T>);

impl<T: ?Sized + Send> Copy for Gc<T> {}
impl<T: ?Sized + Send> Clone for Gc<T> {
    fn clone(&self) -> Self { *self }
}

/// SAFETY: it's only sound to hand out references to the same memory across
///         threads if the underlying type implements `Sync`. Otherwise, all
///         references will be confined to one thread at a time.
unsafe impl<T: ?Sized + Send + Sync> Send for Gc<T> {}
/// SAFETY: Since `Gc<T>` is `Clone + Copy`, conditions on `Sync` are the same.
unsafe impl<T: ?Sized + Send + Sync> Sync for Gc<T> {}

impl<T: ?Sized + Send + Unsize<U>, U: ?Sized + Send> CoerceUnsized<Gc<U>> for Gc<T> {}
impl<T: ?Sized + Send + Unsize<U>, U: ?Sized + Send> DispatchFromDyn<Gc<U>> for Gc<T> {}

/// SAFETY: by all reasonable definitions, the implementation of `Deref for Gc<T>` is "well-behaved" 
unsafe impl<T: ?Sized + Send> DerefPure for Gc<T> {}

impl<T: ?Sized + Send> Deref for Gc<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        // SAFETY: nobody has exclusive access to the inner data, since we don't expose it in the API.
        unsafe { self.0.as_ref() }
    }
}

impl<T: ?Sized + Send> Gc<T> {
    pub fn new(value: T) -> Self where T: Sized {
        let inner = super::allocator::GC_ALLOCATOR.allocate_for_type::<T>().unwrap();
        
        // SAFETY: the memory is aligned and writable, and `T: Send`.
        unsafe { inner.write(mem::MaybeUninit::new(value)); };
        
        // Casting is okay here because we just initialized the data
        Self(inner.cast(), PhantomData)
    }
    
    /// Returns the inner pointer to the value.
    pub fn as_ptr(&self) -> NonNull<T> {
        self.0
    }
    
    /// Constructs a new Gc<T> from a pointer to T.
    /// 
    /// # SAFETY
    /// 
    /// T must already be a pointer to a GC-owned object, with no mutable references/pointers to it.
    /// 
    /// Alternatively, T must be zero-sized, and `value` must be non-null.
    pub unsafe fn from_ptr(value: *const T) -> Self {
        // SAFETY: gauranteed by caller
        let ptr = unsafe { NonNull::new_unchecked(value as *mut T) };
        Self(ptr, PhantomData)
    }
}


/// Exclusive access to Garbage-collected memory.
/// 
/// Having a smart pointer that is not [`Clone`] and which has similar semantics to a
/// [`Box<T>`] allows unconditional mutation without needing any interior mutability
/// types. Similarly, when this type is dropped, it can be marked as "able to free" by the
/// GC immediately, which can save effort in the marking phase.
/// 
/// However, since having an RAII smart pointer for GCed memory is kinda useless on its own,
/// common use patterns for this would be to put data into the GC heap, mutate/initialize it,
/// and then using [`GcMut::demote`] to easily share it between multiple threads. It can also
/// be used in the rare case when needing to [`Send`] the value to another thread without
/// requiring the inner type be [`Sync`].
/// 
/// It should be noted that this type is *very* similar to [`Box<T, GCAllocator>`], and really
/// the only major difference between the two types is the ability to [`demote`] into a shared
/// [`Gc<T>`].
/// 
/// [`demote`]: Self::demote
#[repr(transparent)]
pub struct GcMut<T: ?Sized>(Unique<T>);

// SAFETY: sending a `GcMut` across threads does not need `T: Sync` since it cannot "leave" any references behind.
unsafe impl<T: ?Sized + Send> Send for GcMut<T> {}
// SAFETY: sharing an `&&mut T` is basically the same as `&T`, and so requires `T: Sync`
unsafe impl<T: ?Sized + Sync> Sync for GcMut<T> {}

// SAFETY: the implementation of `Deref for GcMut<T>` is "well-behaved" by any/all definitions
unsafe impl<T: ?Sized> DerefPure for GcMut<T> {}

impl<T: ?Sized + Unsize<U>, U: ?Sized> CoerceUnsized<GcMut<U>> for GcMut<T> {}
impl<T: ?Sized + Unsize<U>, U: ?Sized> DispatchFromDyn<GcMut<U>> for GcMut<T> {}

impl<T: ?Sized> Deref for GcMut<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        // SAFETY: since there is a `&self`, there is no `&mut self`.
        unsafe { self.0.as_ref() }
    }
}

impl<T: ?Sized> std::ops::DerefMut for GcMut<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: since we have an `&mut self`, we know we have the only reference to the inner data
        unsafe { self.0.as_mut() }
    }
}

impl<T: ?Sized> GcMut<T> {
    pub fn new(value: T) -> Self where T: Sized {
        let memory = if std::mem::size_of::<T>() != 0 {
            GC_ALLOCATOR.allocate_for_type::<T>().unwrap()
        } else {
            // SAFETY: all pointers are valid for writes of size 0
            std::ptr::NonNull::dangling()
        };
        
        // SAFETY: the memory is aligned and writable.
        unsafe { memory.write(MaybeUninit::new(value)) };
        
        Self(memory.cast().into())
    }
    
    pub fn new_uninit() -> GcMut<mem::MaybeUninit<T>> where T: Sized {
        Self::try_new_uninit().unwrap()
    }
    
    pub fn try_new_uninit() -> Result<GcMut<mem::MaybeUninit<T>>, GCAllocatorError> where T: Sized {
        Ok(GcMut(super::allocator::GC_ALLOCATOR.allocate_for_type::<T>()?.into()))
    }
    
    /// Returns a pointer to the underlying data.
    /// 
    /// The returned pointer has the same aliasing requirements as [`Box::as_ptr`].
    pub fn as_ptr(&self) -> *const T {
        self.0.as_ptr()
    }
    
    /// Returns a [`NonNull`] pointer to the underlying data.
    /// 
    /// This has the same requirements and caveats as [`Box::as_mut_ptr`].
    /// 
    /// [`NonNull`]: std::ptr::NonNull
    pub fn as_non_null_ptr(&self) -> NonNull<T> {
        self.0.as_non_null_ptr()
    }
    
    /// Constructs a new `GcMut<T>` from a pointer to `T`.
    /// 
    /// SAFETY: `value` must already be a pointer to a GC-owned
    /// object, with no other references/active pointers to it.
    pub unsafe fn from_nonnull_ptr(value: NonNull<T>) -> Self {
        // SAFETY: asserted by caller
        unsafe {
            // NOTE: this isn't really for optimizations, but to have an
            //       `assert_unsafe_precondition` in debug mode
            core::hint::assert_unchecked(super::allocator::GC_ALLOCATOR.contains(value.as_ptr()));
        }
        Self(value.into())
    }
    
    /// Converts exclusive access into shared access.
    /// 
    /// `T` has to be `Send` since unlike a `GcMut`, the data's destructor will be run on the GC thread, and not this one.
    pub fn demote(self) -> Gc<T> where T: Send + 'static {
        // SAFETY: `self.inner` is already GC-ed memory, and does not have any
        //          other references to it (since we moved `self`)
        let val = unsafe { Gc::from_ptr(self.0.as_ptr()) };
        // prevent destructor from running
        std::mem::forget(self);
        val
    }
}

impl<T> GcMut<MaybeUninit<T>> {
    /// See [`Box::assume_init`]
    pub unsafe fn assume_init(self) -> GcMut<T> {
        GcMut(self.0.cast())
    }
    
    /// Writes a value into the [`MaybeUninit`], initializing it
    pub fn write(mut self, value: T) -> GcMut<T> {
        unsafe {
            (*self).write(value);
            self.assume_init()
        }
    }
}

impl<T: ?Sized> Drop for GcMut<T> {
    fn drop(&mut self) {
        // SAFETY: T must be sized on construction, so even if we have been coerced to unsized, its still valid
        let inner_layout = unsafe { Layout::for_value_raw(self.0.as_ptr()) };
        
        // Drop the inner `T`
        unsafe { std::ptr::drop_in_place(self.0.as_ptr()) };
        
        if inner_layout.size() != 0 {
            // SAFETY: if we get here, the GC can definitely free this allocation
            unsafe { GC_ALLOCATOR.deallocate(self.0.as_non_null_ptr().cast(), inner_layout) }
        }
    }
}


#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    
    use super::*;
    
    /// Tests multiple allocations through the GcMut interface
    #[test]
    fn test_multiple_gc_muts() {
        let x: GcMut<[i32]> = GcMut::new([1, 2, 3, 4]);
        let mut y = GcMut::new(vec![(), ()]);
        let z = GcMut::new(0x69696969696969696969696969696969u128);
        
        for _ in 0..5 { y.push(()) };
        
        assert!(x.as_ptr().cast() < y.as_ptr() && y.as_ptr().cast() < z.as_ptr());
    }
    
    /// Tests to make sure that `Drop` is synchronously run for `GcMut`
    #[test]
    fn test_gc_mut_drop() {
        static READY: AtomicBool = AtomicBool::new(false);
        static DATA: Mutex<i32> = Mutex::new(0);
        
        struct WritesOnDrop(i32);
        impl Drop for WritesOnDrop {
            fn drop(&mut self) {
                *DATA.lock().unwrap() = self.0;
                debug!("Dropping `WritesOnDrop({})`", self.0);
                READY.store(true, Ordering::Release);
            }
        }
        
        debug!("Dropping a new `GcMut(WritesOnDrop(69))`");
        // drop(Box::new_in(WritesOnDrop(69), &*GC_ALLOCATOR));
        drop(GcMut::new(WritesOnDrop(69)));
        debug!("Succeeded");
        while READY.compare_exchange(true, true, Ordering::Acquire, Ordering::Relaxed).is_err() {}
        assert_eq!(*DATA.lock().unwrap(), 69);
    }
    
    /// Sends a GCed atomic counter to a bunch of threads, and has them all update it
    #[test]
    fn test_gc_send_atomic() {
        const N: usize = 20;
        const { assert!(N < 64) };
        let counter = Gc::new(AtomicUsize::new(0));
        let handles = (0..N).map(|i| std::thread::spawn(move || {
            counter.fetch_add(1 << i, Ordering::Relaxed);
        }));
        for h in handles { h.join().unwrap() }
        assert_eq!(counter.load(Ordering::Relaxed), (1 << N) - 1);
    }
    
    /// Credit goes to
    /// [Manish Goregaokar](https://manishearth.github.io/blog/2021/04/05/a-tour-of-safe-tracing-gc-designs-in-rust/)
    /// for this example
    #[test]
    #[should_panic]
    #[deny(unsafe_code)]
    fn test_evil_drop() {
        use crate::cell::AtomicRefCell;
        use std::marker::PhantomPinned;
        
        struct LongLived {
            dangle: AtomicRefCell<Option<Gc<CantKillMe>>>
        }
        impl LongLived {
            fn new() -> Self {
                Self { dangle: AtomicRefCell::new(None) }
            }
        }
        
        struct CantKillMe {
            // set up to point to itself during construction
            self_ref: AtomicRefCell<Option<Gc<CantKillMe>>>,
            long_lived: Gc<LongLived>,
            _phantom: PhantomPinned,
        }
        impl CantKillMe {
            fn new(x: Gc<LongLived>) -> Self {
                Self {
                    self_ref: AtomicRefCell::new(None),
                    long_lived: x,
                    _phantom: PhantomPinned
                }
            }
        }
        impl core::fmt::Debug for CantKillMe {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("CantKillMe { ... }")
            }
        }
        
        impl Drop for CantKillMe {
            fn drop(&mut self) {
                // attach self to `long_lived`
                debug!("dropping cantkillme");
                let x: Gc<CantKillMe> = *self.self_ref.try_borrow().unwrap().as_ref().unwrap();
                *self.long_lived.dangle.try_borrow_mut().unwrap() = Some(x);
            }
        }
        
        let long = Gc::new(LongLived::new());
        {
            let cant = Gc::new(CantKillMe::new(long));
            *cant.self_ref.try_borrow_mut().unwrap() = Some(cant);
            // cant goes out of scope, CantKillMe::drop is run
            // cant is attached to long_lived.dangle but still cleaned up
            trace!("evil_drop: Dropped the only live reference to `CantKillMe`");
        }
        
        let start_time = Instant::now();
        while long.dangle.try_borrow_mut().map(|x| x.is_none()).unwrap_or(true) {
            // only wait for three seconds, idk how frequent gc pauses usually are
            if Instant::now() - start_time > Duration::from_secs(1) {
                panic!("`CantKillMe` was not dropped within 1 second")
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        debug!("evil_drop: `CantKillMe` was dropped");
        
        // Dangling reference!
        let x = long.dangle.try_borrow_mut().unwrap();
        let dangle = x.as_deref().unwrap();
        
        println!("Dangling reference: {:?}", dangle);
    }
}
