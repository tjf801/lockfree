use core::{cell::UnsafeCell, sync::atomic::{AtomicBool, Ordering}};

pub struct TakeCell<T: ?Sized> {
    taken: AtomicBool,
    value: UnsafeCell<T>
}

unsafe impl<T: ?Sized + Send> Sync for TakeCell<T> {}

impl<T> TakeCell<T> {
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

impl<T: ?Sized> TakeCell<T> {
    pub fn is_taken(&self) -> bool {
        self.taken.load(Ordering::Relaxed)
    }
    
    pub fn take(&self) -> Option<&mut T> {
        match self.taken.swap(true, Ordering::Relaxed) {
            true => None,
            // SAFETY:
            //    since the ordering of writes to `taken` is total, we know that
            //    only one thread calling `take` concurrently will observe
            //    `false` from the `swap` call, and so it is sound to create a
            //    mutable reference.
            false => Some(unsafe { self.steal() })
        }
    }
    
    /// SAFETY: no other thread can have already taken the inner reference (i.e: `is_taken` returns `false`).
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn steal(&self) -> &mut T {
        let taken = self.taken.swap(true, Ordering::Relaxed);
        // SAFETY: guaranteed by caller.
        unsafe {
            core::hint::assert_unchecked(!taken);
            &mut *self.value.get()
        }
    }
    
    pub fn get_mut(&mut self) -> &mut T {
        // since we have exclusive reference to the whole `TakeCell`, we can
        // get an exclusive reference to the data
        self.value.get_mut()
    }
    
    pub fn heal(&mut self) {
        // since we have exclusive reference to the whole `TakeCell`, nobody can have a reference to the inner value.
        self.taken = AtomicBool::new(false);
    }
}

impl<T: Default> Default for TakeCell<T> {
    fn default() -> Self {
        TakeCell::new(T::default())
    }
}
