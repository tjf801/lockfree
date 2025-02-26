//! Smart pointer types for Garbage Collected (GCed) memory.
//! 
//! This is the main API for interacting with the garbage collector.
//! 
//! TODO: consider potential `Pin<Gc<T>>` APIs?

use std::alloc::{Allocator, Layout};
use std::fmt::{Debug, Display};
use std::marker::{PhantomData, Unsize};
use std::mem::MaybeUninit;
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
pub struct Gc<T: ?Sized + 'static>(NonNull<T>, PhantomData<&'static T>);

impl<T: ?Sized> Copy for Gc<T> {}
impl<T: ?Sized> Clone for Gc<T> {
    fn clone(&self) -> Self { *self }
}

/// SAFETY: it's only sound to hand out references to the same memory across
///         threads if the underlying type implements `Sync`. Otherwise, all
///         references will be confined to one thread at a time.
unsafe impl<T: ?Sized + Sync> Send for Gc<T> {}
/// SAFETY: Since `Gc<T>` is `Clone + Copy`, conditions on `Sync` are the same.
unsafe impl<T: ?Sized + Sync> Sync for Gc<T> {}

impl<T: ?Sized + Unsize<U>, U: ?Sized> CoerceUnsized<Gc<U>> for Gc<T> {}
impl<T: ?Sized + Unsize<U>, U: ?Sized> DispatchFromDyn<Gc<U>> for Gc<T> {}

/// SAFETY: by all reasonable definitions, the implementation of `Deref for Gc<T>` is "well-behaved" 
unsafe impl<T: ?Sized> DerefPure for Gc<T> {}

impl<T: ?Sized> Deref for Gc<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        // SAFETY: nobody has exclusive access to the inner data, since we don't expose it in the API.
        unsafe { self.0.as_ref() }
    }
}

impl<T: ?Sized> Gc<T> {
    /// Moves a value into GCed memory.
    /// 
    /// Requires `T: Send` since the GC thread will gain ownership of the value in order to drop it.
    pub fn new(value: T) -> Self where T: Sized + Send {
        let inner = super::allocator::GC_ALLOCATOR.allocate_for_value(value).map_err(|(e, _)| e).unwrap();
        // Casting is okay here because we just initialized the data
        Self(inner.cast(), PhantomData)
    }
    
    /// Constructs a new Gc<T> from a pointer to T.
    /// 
    /// # Safety
    /// 
    /// T must already be a pointer to a GC-owned object, with no mutable references/pointers to it.
    /// 
    /// Alternatively, T must be zero-sized, and `value` must be non-null.
    pub unsafe fn from_ptr(value: *const T) -> Self {
        // SAFETY: gauranteed by caller
        let ptr = unsafe { NonNull::new_unchecked(value as *mut T) };
        Self(ptr, PhantomData)
    }
    
    /// Promotes the shared pointer into an exclusive pointer.
    /// 
    /// # SAFETY
    /// This function is only safe to call if this is the only GC<T> into the given allocation.
    pub unsafe fn promote(self) -> GcMut<T> {
        unsafe { GcMut::from_nonnull_ptr(self.0) }
    }
    
    /// Runs the destructor of the referenced value, and frees the memory.
    /// 
    /// # SAFETY
    ///  - this must be the only reference to the value, ***including* (potentially dead) cycles**
    ///  - the value must be safe to drop on this thread.
    ///  - TODO: find out more preconditions for this SUPER evil function
    pub unsafe fn drop_unchecked(self) {
        todo!()
    }
    
    /// Returns the inner pointer to the value.
    pub fn as_ptr(&self) -> *const T {
        self.0.as_ptr()
    }
    
    /// Returns the inner pointer to the value.
    pub fn as_non_null_ptr(&self) -> NonNull<T> {
        self.0
    }
    
}

// std trait impls

impl<T: ?Sized + Debug> Debug for Gc<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        <T as Debug>::fmt(self, f)
    }
}

impl<T: ?Sized + Display> Display for Gc<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        <T as Display>::fmt(self, f)
    }
}

impl<T: ?Sized + std::fmt::Pointer> std::fmt::Pointer for Gc<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Pointer::fmt(&**self, f)
    }
}

impl<T: ?Sized + PartialEq> PartialEq for Gc<T> {
    fn eq(&self, other: &Self) -> bool {
        self == other
    }
}

impl<T: ?Sized + Eq> Eq for Gc<T> {}

impl<T: ?Sized + PartialOrd> PartialOrd for Gc<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        (**self).partial_cmp(other)
    }
}

impl<T: ?Sized + Ord> Ord for Gc<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (**self).cmp(other)
    }
}

impl<T: ?Sized + std::hash::Hash> std::hash::Hash for Gc<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (**self).hash(state)
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
    /// Moves a value into GCed memory.
    pub fn new(value: T) -> Self where T: Sized {
        match Self::try_new(value) {
            Err((e, _value)) => panic!("{:?}", e),
            Ok(r) => r,
        }
    }
    
    /// Tries to move the value into GCed memory. 
    /// 
    /// If it fails for whatever reason, it returns the value back with the error.
    pub fn try_new(value: T) -> Result<GcMut<T>, (GCAllocatorError, T)> where T: Sized {
        #[repr(transparent)]
        struct AssertSend<T: ?Sized>(T);
        // SAFETY: The value will still be dropped on this thread (unless it gets demoted, but that needs `Send` anyways)
        unsafe impl<T: ?Sized> Send for AssertSend<T> {}
        
        match GC_ALLOCATOR.allocate_for_value(AssertSend(value)) {
            // NOTE: casting is okay here bc of `#[repr(transparent)]`
            Ok(ptr) => Ok(Self(ptr.cast::<T>().into())),
            Err((e, v)) => Err((e, v.0))
        }
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
    /// # Safety
    /// 
    /// `value` must already be a pointer to a GC-owned object, with no other references/active pointers to it.
    /// Also `value` must not be zero sized.
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
    /// 
    /// # Safety
    /// 
    /// Same as [`Box::assume_init`]
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

unsafe impl<#[may_dangle] T: ?Sized> Drop for GcMut<T> {
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


// std trait impls

impl<T: ?Sized + Debug> Debug for GcMut<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        <T as Debug>::fmt(self, f)
    }
}

impl<T: ?Sized + Display> Display for GcMut<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        <T as Display>::fmt(self, f)
    }
}

impl<T: ?Sized + std::fmt::Pointer> std::fmt::Pointer for GcMut<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Pointer::fmt(&**self, f)
    }
}

impl<T: ?Sized + PartialEq> PartialEq for GcMut<T> {
    fn eq(&self, other: &Self) -> bool {
        self == other
    }
}

impl<T: ?Sized + Eq> Eq for GcMut<T> {}

impl<T: ?Sized + PartialOrd> PartialOrd for GcMut<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        (**self).partial_cmp(other)
    }
}

impl<T: ?Sized + Ord> Ord for GcMut<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (**self).cmp(other)
    }
}

impl<T: ?Sized + std::hash::Hash> std::hash::Hash for GcMut<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (**self).hash(state)
    }
}


// tests

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;
    
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
                println!("Dropping `WritesOnDrop({})`", self.0);
                READY.store(true, Ordering::Release);
            }
        }
        
        println!("Dropping a new `GcMut(WritesOnDrop(69))`");
        // drop(Box::new_in(WritesOnDrop(69), &*GC_ALLOCATOR));
        drop(GcMut::new(WritesOnDrop(69)));
        println!("Succeeded");
        // while READY.compare_exchange(true, true, Ordering::Acquire, Ordering::Relaxed).is_err() {}
        assert_eq!(*DATA.lock().unwrap(), 69);
    }
    
    #[test]
    #[allow(unused_assignments, unused_variables)]
    fn test_covariance() {
        let s = String::from("Hello world!");
        let mut gcmut1 = GcMut::new(&*s);
        let gcmut2: GcMut<&'static str> = GcMut::new("Hello world 2!");
        gcmut1 = gcmut2;
        
        let mut gc1: Gc<dyn Fn(&'static i32) -> &'static i32> = Gc::new(std::convert::identity);
        let gc2: Gc<dyn for<'a> Fn(&'a i32) -> &'a i32> = Gc::new(|x| x);
        gc1 = gc2;
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
    
    #[test]
    fn test_garbage_leak() {
        const NUM_BLOCKS: i32 = 500;
        const HEADER_SIZE: usize = 0x20;
        
        let first = Gc::new(0);
        for i in 1..NUM_BLOCKS {
            let _ = Gc::new([i; 8]);
        }
        
        let size_per_block = HEADER_SIZE + size_of::<[i32; 8]>();
        let expected = first.as_ptr().wrapping_byte_add(size_per_block * (NUM_BLOCKS - 1) as usize);
        
        // Test to make sure that the GC has run to free all the stuff we dropped duiring the loop
        super::GC_ALLOCATOR.wait_for_gc();
        let new = Gc::new(123);
        
        // the new data should reuse old memory
        assert!(new.as_ptr() < expected);
    }
    
    #[test]
    fn test_vec_gc() {
        let vec: Vec<Gc<i32>> = (0..20).map(Gc::new).collect();
        println!("{vec:?}");
        super::GC_ALLOCATOR.wait_for_gc();
        drop(vec);
        super::GC_ALLOCATOR.wait_for_gc();
    }
    
    /// Credit goes to
    /// [Manish Goregaokar](https://manishearth.github.io/blog/2021/04/05/a-tour-of-safe-tracing-gc-designs-in-rust/)
    /// for this example
    /// 
    /// TODO: SOLVE THIS ISSUE. 
    /// 1. Don't allow destructors on GCed types.
    ///     * Frankly, this solution sucks.
    ///     * If this is paired with a `Collect` or `Finalize` trait or similar, that eliminates
    ///       using ANY external type with the GC because of the orphan rule.
    ///     * Also, I don't want to have to write finalizers for all of the `std` types just for
    ///       a few evil Drop implementations that imo, no sane person would ever write
    /// 2. Allow destructors on GCed types, but make sure they don't make dangling pointers
    ///     * Naïvely, this would require a full scan phase for every single destructor run.
    ///     * Also, it would require *aborting the process* if one is found to maintain safety.
    ///     * This also doesn't really account for all of the evil things (e.g starting threads)
    ///       that you *can* do in destructors.
    ///     * This could *probably* be made much easier to implement if you can get dropchk info
    ///       for any given type, but I don't think thats possible.
    ///     * Overall this MASSIVELY slows down the 
    /// 3. Only run destructors for non-cyclically referenced types
    ///     * This is *better*, but it reduces a lot of the advantage of having a GC in the first
    ///       place.
    ///     * For a big network of objects, none of their destructors get run when theyre freed.
    ///     * Maybe there's a way to have like, an *optional* `Finalize` trait, and store that
    ///       info in the heap metadata?
    ///     * TODO: is this actually sound? I feel like there are even nicher cases where this
    ///       could break, even without unsafe code. the `&mut T` in `drop` might be able to
    ///       conjure up a `Gc<T>` that points to `self` somehow, and then stash it somewhere.
    ///       Definitely need to think about this one more, and justify to myself why it works,
    ///       but in the meantime ill just implement it I think.
    #[test]
    #[deny(unsafe_code)]
    fn test_evil_drop() {
        use crate::cell::AtomicRefCell;
        use std::marker::PhantomPinned;
        
        #[derive(Debug)]
        struct LongLived {
            dangle: AtomicRefCell<Option<Gc<CantKillMe>>>
        }
        impl LongLived {
            fn new() -> Self {
                Self { dangle: AtomicRefCell::new(None) }
            }
        }
        
        static DROP_NOTIF: Mutex<bool> = Mutex::new(false);
        
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
                f.write_str(&format!("CantKillMe {{self_ref: {:?}, long_lived: {:?}}}", self.self_ref, self.long_lived))
            }
        }
        
        impl Drop for CantKillMe {
            fn drop(&mut self) {
                // attach self to `long_lived`
                println!("dropping cantkillme (BAD)");
                let x: Gc<CantKillMe> = *self.self_ref.try_borrow().unwrap().as_ref().unwrap();
                *self.long_lived.dangle.try_borrow_mut().unwrap() = Some(x);
                *DROP_NOTIF.lock().unwrap() = true;
            }
        }
        
        let long = Gc::new(LongLived::new());
        {
            let cant = Gc::new(CantKillMe::new(long));
            *cant.self_ref.try_borrow_mut().unwrap() = Some(cant);
            // cant goes out of scope, CantKillMe::drop is run
            // cant is attached to long_lived.dangle but still cleaned up
            debug!("evil_drop: Dropped the only live reference to `CantKillMe`");
        }
        
        // make sure we wipe the reference out of our registers
        assert_eq!(partitions_recursive(40), 37338);
        
        let mut cycles = 0;
        loop {
            // wait for 5 GC cycles, if it doesnt get collected by then it was probably fine idk
            if cycles >= 5 {
                // yay we passed the test, the evil value wasn't ever collected
                return
            }
            if *DROP_NOTIF.lock().unwrap() { break }
            debug!("evil_drop: Waiting for GC...");
            super::GC_ALLOCATOR.wait_for_gc();
            cycles += 1;
        }
        
        // Dangling reference!
        let x = long.dangle.try_borrow_mut().unwrap();
        let dangle = x.as_deref().unwrap();
        
        warn!("Dangling reference: {:016x?}", dangle as *const _);
        panic!("Got a dangling reference: {:016x?}", dangle as *const _)
    }
    
    /// just some unoptimizable busywork for test threads to do
    fn partitions_recursive(n: u64) -> u64 {
        if n == 0 { return 1 }
        if n <= 3 { return n }
        fn pent(n: i64) -> u64 {
            (n*(3*n-1)/2).try_into().unwrap()
        }
        let mut i = 1;
        let mut sum = 0;
        while pent(-2*i) <= n {
            sum += partitions_recursive(n - pent(2*i-1));
            sum += partitions_recursive(n - pent(-(2*i-1)));
            sum -= partitions_recursive(n - pent(2*i));
            sum -= partitions_recursive(n - pent(-2*i));
            i += 1;
        }
        if pent(2*i-1) <= n { sum += partitions_recursive(n - pent(2*i-1)) }
        if pent(-(2*i-1)) <= n { sum += partitions_recursive(n - pent(-(2*i-1))) }
        if pent(2*i) <= n { sum -= partitions_recursive(n - pent(2*i)) }
        assert!(pent(-2*i) > n);
        sum
    }
    
    
}

#[cfg(test)]
mod linked_list_tests {
    use super::*;
    
    // okay but look how simple an immutable linked list implementation becomes. this is awesome!!!
    struct LinkedList<T: Send + 'static> {
        data: T,
        next: Option<Gc<Self>>
    }
    
    impl<T: Send + Sync> LinkedList<T> {
        fn nil(data: T) -> Gc<Self> {
            Gc::new(Self { data, next: None })
        }
        
        fn cons(data: T, tail: Gc<LinkedList<T>>) -> Gc<Self> {
            Gc::new(Self { data, next: Some(tail) })
        }
        
        fn from_iter(values: impl IntoIterator<Item=T>) -> Gc<Self> {
            let mut iter = values.into_iter();
            let mut current = Self::nil(iter.next().unwrap());
            for value in iter {
                current = Self::cons(value, current);
            }
            current
        }
        
        fn append(self: Gc<Self>, values: impl IntoIterator<Item=T>) -> Gc<Self> {
            let iter = values.into_iter();
            let mut current = self;
            for value in iter {
                current = Self::cons(value, current);
            }
            current
        }
        
        fn fold<B, F: FnMut(B, &T) -> B>(self: Gc<Self>, init: B, mut func: F) -> B {
            let mut accum = init;
            let mut curr_node = self;
            loop {
                accum = func(accum, &curr_node.data);
                match curr_node.next {
                    None => return accum,
                    Some(next) => curr_node = next
                }
            }
        }
    }
    
    struct LLIter<'ll, T: Send + Sync + 'static>(Option<&'ll Gc<LinkedList<T>>>);
    impl<'ll, T: Send + Sync> Iterator for LLIter<'ll, T> {
        type Item = &'ll T;
        fn next(&mut self) -> Option<Self::Item> {
            let result = &self.0?.data;
            self.0 = self.0?.next.as_ref();
            Some(result)
        }
    }
    impl<'gc, T: Send + Sync> IntoIterator for &'gc Gc<LinkedList<T>> {
        type Item = &'gc T;
        type IntoIter = LLIter<'gc, T>;
        fn into_iter(self) -> Self::IntoIter {
            LLIter(Some(self))
        }
    }
    
    #[test]
    fn test_linked_list() {
        println!("{:?}", Gc::new(3));
        
        let x = LinkedList::from_iter(0..10);
        let mut current: Gc<LinkedList<i32>> = x.append(10..20);
        for i in (0..20).rev() {
            assert_eq!(current.data, i);
            if current.next.is_some() {
                current = current.next.unwrap();
            }
        }
        
        current = LinkedList::nil(0);
        super::GC_ALLOCATOR.wait_for_gc();
        std::hint::black_box(current);
    }
    
    #[test]
    fn test_ll_fold() {
        let l = LinkedList::from_iter(0..100);
        assert_eq!(l.fold(0, |x, y| x + y), 99 * 50);
    }
}
