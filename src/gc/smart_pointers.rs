use std::alloc::{Allocator, Layout};
use std::marker::PhantomData;
use std::ops::Deref;
use std::ptr::NonNull;
use std::sync::OnceLock;

use super::allocator::GC_ALLOCATOR;


/// Shared access to GCed memory.
/// 
/// A smart pointer to data that is owned by the garbage collector. This type is similar to an [`Arc`], in
/// that it is basically just a pointer to some data, but differs in the fact that the bookkeeping necessary
/// for knowing when to free the data is done seperately. This allows the type to not only be `Clone`, but
/// since the GC will scan the stack and heap for the pointers, 
/// 
/// This type is also `Copy` since unlike an `Arc`, it does not require any bookkeeping overhead to `clone`.
/// 
/// [`Arc`]: std::sync::Arc
/// [`Copy`]
#[derive(Clone, Copy)]
pub struct Gc<T: ?Sized> {
    inner: NonNull<T>,
}

impl<T: Send> Gc<T> {
    /// `T` must be `Send` to create this, since the thread that is allocating the memory may not be the same one that frees it.
    pub fn new(value: T) -> Self {
        let layout = Layout::for_value(&value);
        let inner = GC_ALLOCATOR.allocate(layout).unwrap().cast::<T>();
        
        // SAFETY: the memory is aligned and writable, and `T: Send`.
        unsafe { inner.write(value) };
        
        Self { inner }
    }
    
    /// SAFETY: `inner` must be a pointer to an object managed by the GC.
    pub unsafe fn from_inner(inner: NonNull<T>) -> Self {
        Self { inner }
    }
}

impl<T: ?Sized> Deref for Gc<T> {
    type Target = T;
    
    fn deref(&self) -> &Self::Target {
        unsafe { self.inner.as_ref() }
    }
}

// SAFETY: same constraints as an `Arc`.
unsafe impl<T: ?Sized + Send + Sync> Send for Gc<T> {}
unsafe impl<T: ?Sized + Send + Sync> Sync for Gc<T> {}


/// Wrapper for *exclusive* access to GCed memory.
/// 
/// `T` must be `Send` because the thread that allocates it might not be the same one that frees it.
pub struct GcMut<T: ?Sized + 'static> {
    inner: NonNull<T>,
    _phantom: PhantomData<&'static mut T>
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
    
    // Convert exclusive access of a GCed memory allocation into a shared access.
    pub fn demote(self) -> Gc<T> {
        let val = unsafe { Gc::from_inner(self.inner) };
        std::mem::forget(self);
        val
    }
}

impl<T: ?Sized> Deref for GcMut<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        // SAFETY: we know there aren't any exclusive references, since this API doesn't expose any.
        unsafe { self.inner.as_ref() }
    }
}

impl<T: ?Sized> std::ops::DerefMut for GcMut<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: since self is borrowed exclusively and is not `Clone`, we have the only reference to the inner data, so we can take a mutable reference.
        unsafe { self.inner.as_mut() }
    }
}

unsafe impl<T: ?Sized + Send> Send for GcMut<T> {}
unsafe impl<T: ?Sized + Send + Sync> Sync for GcMut<T> {}

impl<T: ?Sized> Drop for GcMut<T> {
    fn drop(&mut self) {
        // SAFETY: we have an exclusive reference, so making a shared ref is okay
        let inner_layout = Layout::for_value(unsafe { self.inner.as_ref() });
        
        // SAFETY: if we get here, the GC can definitely free this allocation
        unsafe { GC_ALLOCATOR.deallocate(self.inner.cast(), inner_layout) }
    }
}
