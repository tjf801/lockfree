use core::ops::{Deref, DerefMut, DerefPure};
use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::sync::atomic::{AtomicBool, Ordering};


// ngl i came up with this idea at like 9:30 in the morning on 2024-09-29 and made it in like an hour and a half ._.
/// A lightweight concurrency primitive that only hands out mutable references to the inner value.
/// 
/// (Basically it's a mutex that just gives out an option instead of locking.
/// Alternatively, it's a `TakeCell` with a guard instead of a raw mutable reference.)
pub struct MutCell<T: ?Sized> {
    taken: AtomicBool,
    value: UnsafeCell<T>
}

/// SAFETY: In order to share a `MutCell`, the other thread might `mem::swap` the inner value, and so might be sent across the thread.
///         However, since only one thread ever has access to the value at the same time, `Sync` is not needed, since one could view
///         this type as repeatedly `Send`ing the inner data between the different threads. This is also similar to a `Mutex`.
unsafe impl<T: ?Sized + Send> Sync for MutCell<T> {}

impl<T: Sized> MutCell<T> {
    pub const fn new(value: T) -> Self {
        Self {
            taken: AtomicBool::new(false),
            value: UnsafeCell::new(value)
        }
    }
    
    pub const fn into_inner(self) -> T {
        self.value.into_inner()
    }
}

impl<T: ?Sized> MutCell<T> {
    /// Given an exclusive reference to the `MutCell`, you can trivially have an exclusive reference to the inner value.
    pub const fn get_mut(&mut self) -> &mut T {
        self.value.get_mut()
    }
    
    /// Reset the `MutCell` to the default (unborrowed) state.
    /// 
    /// This can be useful if someone else used `mem::forget` on the guard.
    /// 
    /// This is okay because if we have an exclusive reference to the `MutCell`,
    /// we know that nobody else can have any references to the inner data.
    pub fn heal(&mut self) {
        *self.taken.get_mut() = false;
    }
    
    /// Whether the `MutCell` is actively borrowed.
    pub fn is_taken(&self) -> bool {
        self.taken.load(Ordering::Acquire) // would `Ordering::Consume` be good here?
    }
    
    /// Return a mutable guard to the cell's contents.
    /// 
    /// SAFETY: Caller must ensure that no other references exist, i.e: `!self.taken.load(Ordering::Acquire)`
    pub unsafe fn take_unchecked(&self) -> MutCellGuard<'_, T> {
        // SAFETY: asserted by caller
        unsafe { core::hint::assert_unchecked(!self.taken.swap(true, Ordering::Acquire)) };
        MutCellGuard { inner: self, _phantom: PhantomData }
    }
    
    /// Try to take exclusive access to the inner value.
    pub fn take(&self) -> Option<MutCellGuard<'_, T>> {
        match self.taken.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed) {
            // NOTE: the only time we construct a `MutCellGuard` is when we know `self.value` was `false`
            Ok(_) => Some(MutCellGuard { inner: self, _phantom: PhantomData }),
            Err(_) => None
        }
    }
}


pub struct MutCellGuard<'cell, T: ?Sized> {
    // NOTE: the critical invariant of this type is that no other `MutCellGuard`s with a reference to `inner` exist at the same time.
    inner: &'cell MutCell<T>,
    _phantom: PhantomData<&'cell mut T>
}

// unsafe impl<T: ?Sized + Sync> Sync for MutCellGuard<'_, T> {}

impl<T: ?Sized> Deref for MutCellGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        // SAFETY: the existence of this type means we have exclusive access to the inner value.
        unsafe { &*self.inner.value.get() }
    }
}

impl<T: ?Sized> DerefMut for MutCellGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: the existence of this type means we have exclusive access to the inner value.
        unsafe { &mut *self.inner.value.get() }
    }
}

unsafe impl<T: ?Sized> DerefPure for MutCellGuard<'_, T> {}

impl<T: ?Sized> Drop for MutCellGuard<'_, T> {
    fn drop(&mut self) {
        // NOTE: failing to drop the `MutCellGuard` only holds the lock forever,
        //       which doesn't impact safety. (It will only cause a deadlock.)
        //       In a perfect world, rust would have unleakable types, and this would be one of them.
        let old_value = self.inner.taken.swap(false, Ordering::Release);
        debug_assert!(old_value, "Dropped MutCellGuard without `taken` having been set");
    }
}

