#![allow(unused_attributes)]
#![no_std]

use core::cell::SyncUnsafeCell;
use core::sync::atomic::{AtomicIsize, Ordering};
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut, DerefPure};

/// A thread-safe [`RefCell`].
/// 
/// Alternatively, a `#[no_std]` and lock-free [`RwLock`].
/// 
/// This type dynamically enforces rust's "Aliasing XOR mutability" rule, and
/// uses atomic operations to ensure it happens safely across threads. However,
/// when failing to acquire a reference to the data, it behaves differently.
/// Unlike a [`RefCell`], it does not panic by default, and unlike an [`RwLock`],
/// it does not block.
/// 
/// [`RefCell`]: core::cell::RefCell
/// [`RwLock`]: std::sync::RwLock
pub struct AtomicRefCell<T: ?Sized> {
    borrows: AtomicIsize,
    value: SyncUnsafeCell<T>
}

// SAFETY: Since an &AtomicRefCell<T> can be used to move the inner value across thread boundaries, T must be Send. 
//         And since an &AtomicRefCell<T> can be used to send `&T`s across threads, T must be Sync.
unsafe impl<T: ?Sized + Send + Sync> Sync for AtomicRefCell<T> {}

impl<T> AtomicRefCell<T> {
    pub const fn new(value: T) -> Self {
        AtomicRefCell {
            borrows: AtomicIsize::new(0),
            value: SyncUnsafeCell::new(value)
        }
    }
    
    pub const fn into_inner(self) -> T {
        self.value.into_inner()
    }
}

impl<T: ?Sized> AtomicRefCell<T> {
    pub fn get_mut(&mut self) -> &mut T {
        self.value.get_mut()
    }
    
    pub fn clear_borrows(&mut self) {
        *self.borrows.get_mut() = 0;
    }
    
    pub fn active_borrows(&self) -> isize {
        todo!()
    }
    
    pub fn try_borrow(&self) -> Result<AtomicRef<'_, T>, BorrowError> {
        match self.borrows.fetch_update(Ordering::Acquire, Ordering::Relaxed, |value| {
            if value == isize::MAX { panic!("AtomicRefCell borrow counter overflowed.") }
            if value >= 0 { Some(value + 1) } else { None }
        }) {
            Ok(_) => Ok(AtomicRef { inner: self, _phantom: PhantomData }),
            Err(_) => Err(BorrowError::BorrowedExclusive)
        }
    }
    
    pub fn try_borrow_mut(&self) -> Result<AtomicRefMut<'_, T>, BorrowError> {
        match self.borrows.compare_exchange(0, -1, Ordering::Acquire, Ordering::Relaxed) {
            Ok(_) => Ok(AtomicRefMut{ inner: self, _phantom: PhantomData }),
            Err(_num_borrows) => {
                if _num_borrows > 0 {
                    Err(BorrowError::BorrowedShared)
                } else {
                    Err(BorrowError::BorrowedExclusive)
                }
            },
        }
    }
}

#[derive(core::fmt::Debug)]
pub enum BorrowError {
    /// Attempted to exclusively borrow an [`AtomicRefCell`] when other shared references to it existed.
    BorrowedShared,
    /// Attempted to borrow an [`AtomicRefCell`] while an exclusive reference to it already existed.
    BorrowedExclusive,
}


/// An RAII structure used to manage shared access to an [`AtomicRefCell`].
pub struct AtomicRef<'b, T: ?Sized> {
    inner: &'b AtomicRefCell<T>,
    _phantom: PhantomData<&'b T>
}

impl<'b, T: ?Sized> AtomicRef<'b, T> {
    /// Attempt to upgrade this [`AtomicRef`] into an [`AtomicRefMut`] if able.
    /// 
    /// This can only succeed if this is the only Ref to this [`AtomicRefCell`].
    /// If any other references exist, it will return `Err(self)`.
    pub fn upgrade(value: Self) -> Result<AtomicRefMut<'b, T>, AtomicRef<'b, T>> {
        match value.inner.borrows.compare_exchange(1, -1, Ordering::AcqRel, Ordering::Relaxed) {
            Ok(_) => Ok(AtomicRefMut{ inner: value.inner, _phantom: PhantomData }),
            Err(_) => Err(value)
        }
    }
}

impl<T: ?Sized> Clone for AtomicRef<'_, T> {
    fn clone(&self) -> Self {
        self.inner.borrows.
            fetch_update(Ordering::Acquire, Ordering::Relaxed, |value| {
                if value == isize::MAX || value < 0 { None }
                else { Some(value + 1) }
            })
            .expect("AtomicRefCell borrow counter overflowed.");
        AtomicRef { inner: self.inner, _phantom: PhantomData }
    }
}

impl<T: ?Sized> Deref for AtomicRef<'_, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        // SAFETY: the existence of this type means that nobody can be mutating the value
        unsafe { &*self.inner.value.get() }
    }
}

unsafe impl<T> DerefPure for AtomicRef<'_, T> {}

impl<T: ?Sized> Drop for AtomicRef<'_, T> {
    fn drop(&mut self) {
        self.inner.borrows.fetch_sub(-1, Ordering::Release);
    }
}


/// An RAII structure used to manage exclusive access to an [`AtomicRefCell`].
pub struct AtomicRefMut<'b, T: ?Sized> {
    inner: &'b AtomicRefCell<T>,
    _phantom: PhantomData<&'b mut T>
}

impl<T: ?Sized> Deref for AtomicRefMut<'_, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.inner.value.get() }
    }
}

impl<T: ?Sized> DerefMut for AtomicRefMut<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: we know we have exclusive access while this type exists
        unsafe { &mut *self.inner.value.get() }
    }
}

unsafe impl<T> DerefPure for AtomicRefMut<'_, T> {}

impl<T: ?Sized> Drop for AtomicRefMut<'_, T> {
    fn drop(&mut self) {
        // NOTE: if compare_exchange does not give -1, something went horribly wrong.
        self.inner.borrows
            .compare_exchange(-1, 0, Ordering::Release, Ordering::Relaxed)
            .expect("Borrow counter should be set to -1 for the entire lifetime of the `AtomicRefMut`.");
    }
}

