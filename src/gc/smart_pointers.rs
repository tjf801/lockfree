//! Smart pointer types for Garbage Collected (GCed) memory.
//! 
//! This is the main API for interacting with the garbage collector.
//! 
//! TODO: consider a `GcPin` pointer?

use std::alloc::{Allocator, Layout};
use std::clone::CloneToUninit;
use std::marker::{PhantomData, Unsize};
use std::mem;
use std::ops::{CoerceUnsized, Deref, DerefPure, DispatchFromDyn};
use std::ptr::NonNull;

use super::allocator::GC_ALLOCATOR;


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
pub struct Gc<T: ?Sized + Send + 'static> {
    inner: NonNull<T>,
    _phantom: PhantomData<&'static T>
}

impl<T: ?Sized + Send> Clone for Gc<T> {
    fn clone(&self) -> Self {
        Gc { inner: self.inner, _phantom: PhantomData }
    }
}

impl<T: ?Sized + Send> Copy for Gc<T> {}

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
        unsafe { self.inner.as_ref() }
    }
}

impl<T: Send> Gc<T> {
    pub fn new(value: T) -> Self {
        let layout = Layout::for_value(&value);
        let inner = super::allocator::GC_ALLOCATOR.allocate(layout).unwrap().cast::<T>();
        
        // SAFETY: the memory is aligned and writable, and `T: Send`.
        unsafe { inner.write(value); };
        
        Self { inner , _phantom: PhantomData }
    }
    
    pub fn new_uninit() -> Gc<mem::MaybeUninit<T>> {
        todo!()
    }
    
    pub fn new_zeroed() -> Gc<mem::MaybeUninit<T>> {
        todo!()
    }
}

impl<T: ?Sized + Send> Gc<T> {
    /// Returns the inner pointer to the value.
    pub fn as_ptr(&self) -> NonNull<T> {
        self.inner
    }
    
    /// Constructs a new Gc<T> from a pointer to T.
    /// 
    /// SAFETY: T must already be a pointer to a GC-owned object, with no mutable references/pointers to it.
    pub unsafe fn from_ptr(value: *const T) -> Self {
        Self {
            // SAFETY: gauranteed by caller
            inner: unsafe { NonNull::new_unchecked(value as *mut T) },
            _phantom: PhantomData
        }
    }
}

impl<T: Send> Gc<[T]> {
    /// Creates a new slice of uninitialized memory given a length.
    pub fn new_uninit_slice(len: usize) -> Gc<[mem::MaybeUninit<T>]> {
        let layout = unsafe { Layout::for_value_raw(std::ptr::slice_from_raw_parts(std::ptr::null::<T>(), len)) };
        let bytes_ptr = GC_ALLOCATOR.allocate(layout).unwrap();
        let result_ptr = NonNull::<[mem::MaybeUninit<T>]>::slice_from_raw_parts(bytes_ptr.cast(), len);
        
        Gc { inner: result_ptr, _phantom: PhantomData }
    }
    
    /// [`Clone`]s the contents of a slice into GC-managed memory.
    pub fn clone_from_slice(value: &[T]) -> Self where T: Clone {
        let layout = Layout::for_value(value);
        let bytes_ptr = GC_ALLOCATOR.allocate(layout).unwrap();
        let result_ptr = NonNull::slice_from_raw_parts(bytes_ptr.cast::<T>(), value.len());
        
        // SAFETY: result_ptr has the same length as `value`
        unsafe { value.clone_to_uninit(result_ptr.as_ptr()) };
        
        Self { inner: result_ptr, _phantom: PhantomData }
    }
}


/// Exclusive access to Garbage-collected memory.
/// 
/// Having a smart pointer that is not [`Clone`] and which has similar semantics to a
/// `&mut T` reference allows unconditional mutation without needing any interior mutability
/// types. Similarly, when this type is dropped, it can be marked as "able to free" by the
/// GC immediately, and save effort in the marking phase.
/// 
/// However, since having an RAII smart pointer for GCed memory is kinda useless on its own,
/// common use patterns for this would be to put data into the GC, mutate/initialize it, and
/// then using [`GcMut::demote`] to easily share it between multiple threads. It can also be
/// used in the rare case when needing to [`Send`] the value to another thread, without
/// requiring the inner type be [`Sync`].
/// 
/// `T` must be `Send` because the thread that allocates it will not be the same one that frees it.
pub struct GcMut<T: ?Sized + Send + 'static> {
    /// TODO: consider using [`std::ptr::Unique`]
    inner: NonNull<T>,
    _phantom: PhantomData<&'static mut T>
}

// SAFETY: sending a `GcMut` across threads does not need `T: Sync` since it cannot "leave" any references behind.
unsafe impl<T: ?Sized + Send> Send for GcMut<T> {}
// SAFETY: sharing an `&&mut T` is basically the same as `&T`, and so requires `T: Sync`
unsafe impl<T: ?Sized + Send + Sync> Sync for GcMut<T> {}

// SAFETY: the implementation of `Deref for GcMut<T>` is "well-behaved" by any/all definitions
unsafe impl<T: ?Sized + Send> DerefPure for GcMut<T> {}

impl<T: ?Sized + Send + Unsize<U>, U: ?Sized + Send> CoerceUnsized<GcMut<U>> for GcMut<T> {}
impl<T: ?Sized + Send + Unsize<U>, U: ?Sized + Send> DispatchFromDyn<GcMut<U>> for GcMut<T> {}

impl<T: ?Sized + Send> Deref for GcMut<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        // SAFETY: since there is a `&self`, there is no `&mut self`.
        unsafe { self.inner.as_ref() }
    }
}

impl<T: ?Sized + Send> std::ops::DerefMut for GcMut<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: since we have an `&mut self`, we know we have the only reference to the inner data
        unsafe { self.inner.as_mut() }
    }
}

impl<T: ?Sized + Send> Drop for GcMut<T> {
    fn drop(&mut self) {
        let inner_layout = Layout::for_value(&*self);
        
        // SAFETY: if we get here, the GC can definitely free this allocation
        unsafe { GC_ALLOCATOR.deallocate(self.inner.cast(), inner_layout) }
    }
}

impl<T: Send> GcMut<T> {
    pub fn new(value: T) -> Self {
        let layout = Layout::for_value(&value);
        let memory = GC_ALLOCATOR.allocate(layout).unwrap().cast::<T>();
        
        // SAFETY: the memory is aligned and writable.
        unsafe { memory.write(value) };
        
        Self {
            inner: memory,
            _phantom: PhantomData
        }
    }
    
    /// Converts exclusive access into shared access.
    pub fn demote(self) -> Gc<T> {
        // SAFETY: `self.inner` is already GC-ed memory, and does not have any
        //          other references to it (since we moved `self`)
        let val = unsafe { Gc::from_ptr(self.inner.as_ptr()) };
        std::mem::forget(self);
        val
    }
}

impl<T: ?Sized + Send> GcMut<T> {
    pub fn as_ptr(&self) -> NonNull<T> {
        self.inner
    }
    
    /// Constructs a new `GcMut<T>` from a pointer to `T`.
    /// 
    /// SAFETY: `value` must already be a pointer to a GC-owned
    /// object, with no other references/active pointers to it.
    pub unsafe fn from_ptr(value: NonNull<T>) -> Self {
        // SAFETY: asserted by caller
        unsafe {
            core::hint::assert_unchecked(
                super::allocator::GC_ALLOCATOR
                  .manages_ptr(value.as_ptr())
            )
        };
        Self {
            inner: value,
            _phantom: PhantomData
        }
    }
}

impl<T: Send> GcMut<[T]> {
    /// Creates a new slice of uninitialized memory given a length.
    pub fn new_uninit_slice(len: usize) -> GcMut<[mem::MaybeUninit<T>]> {
        let layout = unsafe { Layout::for_value_raw(std::ptr::slice_from_raw_parts(std::ptr::null::<T>(), len)) };
        let bytes_ptr = GC_ALLOCATOR.allocate(layout).unwrap();
        let result_ptr = NonNull::<[mem::MaybeUninit<T>]>::slice_from_raw_parts(bytes_ptr.cast(), len);
        
        GcMut { inner: result_ptr, _phantom: PhantomData }
    }
    
    /// [`Clone`]s the contents of a slice into GC-managed memory.
    pub fn clone_from_slice(value: &[T]) -> Self where T: Clone {
        let layout = Layout::for_value(value);
        let bytes_ptr = GC_ALLOCATOR.allocate(layout).unwrap();
        let result_ptr = NonNull::slice_from_raw_parts(bytes_ptr.cast::<T>(), value.len());
        
        // SAFETY: result_ptr has the same length as `value`
        unsafe { value.clone_to_uninit(result_ptr.as_ptr()) };
        
        Self { inner: result_ptr, _phantom: PhantomData }
    }
}

#[test]
fn test() {
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
            // attach self to long_lived
            println!("dropping cantkillme");
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
        println!("got here 1 ");
        println!("got here 2 ");
    }
    
    // Dangling reference!
    let x = long.dangle.try_borrow_mut().unwrap();
    let dangle = x.as_deref().unwrap();
    
    println!("{:?}", dangle);
}
