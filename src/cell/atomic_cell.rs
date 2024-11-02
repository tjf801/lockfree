use std::marker::PhantomData;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicPtr, Ordering};


/// TODO: this should really be PhantomData<&'data own T> but alas we cant have nice things
#[repr(transparent)]
pub struct AtomicCell<'data, T>(AtomicPtr<T>, PhantomData<(T, &'data ())>);

unsafe impl<T: Send> Send for AtomicCell<'_, T> {}
unsafe impl<T: Send> Sync for AtomicCell<'_, T> {}

impl<'data, T> AtomicCell<'data, T> {
    pub fn from_mut(value: &'data mut T) -> Self {
        Self(AtomicPtr::new(value as *mut T), PhantomData)
    }
    
    pub fn get(&self) -> T where T: Copy {
        unsafe { self.0.load(Ordering::Acquire).read() }
    }
    
    pub fn replace(&self, value: &'data mut T) -> Option<&'data mut T> {
        let ptr = self.0.swap(value, Ordering::AcqRel);
        unsafe { Some(NonNull::new(ptr)?.as_mut()) }
    }
    
    pub fn take(&self) -> Option<&'data mut T> {
        let ptr = self.0.swap(std::ptr::null_mut(), Ordering::AcqRel);
        unsafe { Some(NonNull::new(ptr)?.as_mut()) }
    }
    
    pub fn get_mut<'a>(&'a mut self) -> &'a mut Option<&'data mut T> {
        // NOTE: returning a &mut *mut T is unsound since you can set it to a dangling
        // pointer, but then calling any other method would dereference it
        
        // SAFETY: trust me bro
        unsafe { std::mem::transmute(self.0.get_mut()) }
    }
    
    pub fn into_inner(self) -> Option<&'data mut T> {
        unsafe { self.0.into_inner().as_mut() }
    }
}
