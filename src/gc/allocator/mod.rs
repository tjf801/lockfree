use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::ptr::NonNull;
use std::alloc::{Allocator, Layout};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::LazyLock;

use super::os_dependent::windows::mem_source::WindowsMemorySource;
use super::os_dependent::MemorySource;


#[non_exhaustive]
#[derive(Debug, Clone, Copy)]
pub enum GCAllocatorError {
    ZeroSized,
    AlignmentTooHigh,
    OutOfMemory,
}

type HeaderFlag = usize;
const HEADERFLAG_NONE: HeaderFlag = 0x00;
/// whether the heap block is allocated
const HEADERFLAG_ALLOCATED: HeaderFlag = 0x01;
/// Current heap block is marked as unreferenced, and can be dropped
const HEADERFLAG_OK_TO_DROP: HeaderFlag = 0x02;
/// Current heap block has been dropped and can be deallocated
const HEADERFLAG_DROPPED: HeaderFlag = 0x04;

/// NOTE: this struct also owns `self.size` contiguous bytes after it in memory.
#[repr(C, align(16))]
struct GCHeapBlockHeader {
    next: Option<NonNull<GCHeapBlockHeader>>,
    size: usize,
    flags: HeaderFlag,
    drop_in_place: Option<unsafe fn(*mut ())>,
}

impl GCHeapBlockHeader {
    fn is_allocated(&self) -> bool {
        self.flags & HEADERFLAG_ALLOCATED != 0
    }
    
    fn mark_allocated(&mut self) {
        assert!(!self.is_allocated());
        self.flags |= HEADERFLAG_ALLOCATED
    }
    
    fn data(&self) -> NonNull<[u8]> {
        let ptr = unsafe { NonNull::from(self).cast::<()>().byte_add(size_of::<Self>()) };
        let len = self.size;
        NonNull::from_raw_parts(ptr, len)
    }
    
    fn can_allocate(&self, layout: Layout) -> bool {
        if self.is_allocated() { return false }
        
        // check size
        if self.size < layout.size() {
            return false
        }
        
        // check alignment
        if (<*const _>::addr(self) + size_of::<Self>()) & (layout.align() - 1) != 0 {
            return false
        }
        
        true
    }
    
    // truncates the block to fit at least `size` bytes, and updates the `next` pointer to point to the new block in this block's space.
    fn truncate_and_split(&mut self, num_bytes: usize) -> Result<NonNull<Self>, ()> {
        let truncated_size = num_bytes.next_multiple_of(align_of::<Self>());
        
        if self.size < truncated_size + size_of::<Self>() {
            return Err(())
        }
        
        // SAFETY: the truncated size is within this block's data
        let new_block_ptr = unsafe { self.data().cast::<Self>().byte_add(truncated_size) };
        let new_block_size = self.size - truncated_size - size_of::<Self>();
        
        // initialize the new block
        unsafe {
            let ptr = new_block_ptr.as_ptr();
            (&raw mut (*ptr).next).write(self.next);
            (&raw mut (*ptr).size).write(new_block_size);
            (&raw mut (*ptr).flags).write(HEADERFLAG_NONE);
            (&raw mut (*ptr).drop_in_place).write(None);
        }
        
        // update this block's 'next' pointer
        self.next = Some(new_block_ptr);
        // update this block's size
        self.size = truncated_size;
        
        Ok(new_block_ptr)
    }
}


// TODO: tri-color allocations
pub struct GCAllocator<M: 'static + Sync + MemorySource> {
    memory_source: &'static M,
    gc_thread: std::thread::JoinHandle<()>,
    heap_lock: AtomicU8,
    heap_freelist_head: UnsafeCell<Option<NonNull<GCHeapBlockHeader>>>,
}

// TODO: actually make sure this is thread-safe lmao
unsafe impl<M: Sync + MemorySource> Send for GCAllocator<M> {}
unsafe impl<M: Sync + MemorySource> Sync for GCAllocator<M> {}


struct HeapLock<'a>(&'a AtomicU8);
impl<'a> HeapLock<'a> {
    fn new(lock: &'a AtomicU8) -> Self {
        // TODO: use an actually good lock, either in `std::sys` or some other library
        loop {
            // wait until we load a 0
            while lock.load(Ordering::Relaxed) != 0 { std::hint::spin_loop() }
            match lock.compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed) {
                    Ok(_) => { break Self(lock) },
                    Err(_) => { } // someone else got here before us
            }
        }
    }
}
impl Drop for HeapLock<'_> {
    fn drop(&mut self) {
        // release the lock
        match self.0.compare_exchange(1, 0, Ordering::Release, Ordering::Relaxed) {
            Err(_) => panic!("We set it to 1"),
            Ok(_) => {}
        }
    }
}


impl<M> GCAllocator<M> where M: Sync + MemorySource {
    fn new(memory_source: &'static M) -> Self {
        // TODO: make some sort of `gc_main() -> !` function
        Self {
            memory_source,
            gc_thread: std::thread::spawn(|| eprintln!("TODO: actual garbage collecting thread")),
            heap_lock: AtomicU8::new(0),
            heap_freelist_head: UnsafeCell::new(None)
        }
    }
    
    fn lock_heap(&self) -> HeapLock {
        HeapLock::new(&self.heap_lock)
    }
    
    // Expands the heap by at least the given number of bytes, and returns the block
    fn expand_heap(&self, num_bytes: usize, _lock: &HeapLock) -> Result<NonNull<GCHeapBlockHeader>, GCAllocatorError> {
        // allocate some space
        let page_size = self.memory_source.page_size();
        let num_pages = num_bytes.div_ceil(page_size);
        let new_ptr = self.memory_source.grow_by(num_pages).ok_or(GCAllocatorError::OutOfMemory)?;
        
        // update the new heap block
        let new_block = new_ptr.cast::<GCHeapBlockHeader>();
        
        // SAFETY: we own this data, its fine to write
        unsafe {
            let pointer = new_block.as_ptr();
            let block_len = new_ptr.len() - size_of::<GCHeapBlockHeader>();
            (&raw mut (*pointer).next).write(None);
            (&raw mut (*pointer).size).write(block_len);
            (&raw mut (*pointer).flags).write(HEADERFLAG_NONE);
            (&raw mut (*pointer).drop_in_place).write(None);
        }
        
        Ok(new_ptr.cast())
    }
    
    fn find_and_allocate_allocable_block(&self, layout: Layout, _lock: &HeapLock) -> Result<(NonNull<GCHeapBlockHeader>, NonNull<[u8]>), GCAllocatorError> {
        // TODO: support bigger alignments than 16
        if layout.align() > 16 {
            return Err(GCAllocatorError::AlignmentTooHigh)
        }
        
        // traverse the free list looking for a block
        let mut previous_block: Option<NonNull<GCHeapBlockHeader>> = None;
        let mut current_block = unsafe { *self.heap_freelist_head.get() }.expect("should have just set to Some()");
        loop {
            // SAFETY: TODO
            let current_block_ref = unsafe { current_block.as_mut() };
            assert!(!current_block_ref.is_allocated(), "block@{:x?} is allocated", current_block_ref as *const _);
            
            if current_block_ref.can_allocate(layout) {
                // split off excess memory if it's big enough
                let _ = current_block_ref.truncate_and_split(layout.size());
                current_block_ref.mark_allocated();
                break; // we found a block
            }
            
            match current_block_ref.next {
                Some(next_block) => {
                    previous_block = Some(current_block);
                    current_block = next_block;
                }
                None => {
                    // TODO: grow memory, since we reached the end of the heap
                    todo!()
                }
            }
        }
        
        let result_block = unsafe { &mut *current_block.as_ptr() };
        match previous_block {
            None => {
                let first = unsafe { *self.heap_freelist_head.get() };
                // first entry in the heap list
                assert_eq!(first.unwrap().as_ptr() as *const _, result_block as *const GCHeapBlockHeader);
                unsafe { *self.heap_freelist_head.get() = result_block.next };
            }
            Some(previous_block) => unsafe {
                // pop out `result_block` from the linked list
                (*previous_block.as_ptr()).next = result_block.next
            }
        }
        result_block.next = None;
        result_block.mark_allocated();
        
        Ok((result_block.into(), result_block.data()))
    }
    
    /// Allocates at least `layout.size()` bytes with alignment of at least `layout.align()`.
    unsafe fn raw_allocate(&self, layout: Layout) -> Result<(NonNull<GCHeapBlockHeader>, NonNull<[u8]>), GCAllocatorError> {
        let lock = self.lock_heap();
        
        // make sure we actually have some free memory lol
        // SAFETY: we hold the heap lock so this is fine
        if unsafe { (*self.heap_freelist_head.get()).is_none() } {
            let result = self.expand_heap(layout.size(), &lock)?;
            // SAFETY: TODO
            unsafe { self.heap_freelist_head.get().write(Some(result)) }
        }
        
        let result = self.find_and_allocate_allocable_block(layout, &lock);
        
        result
    }
    
    /// allocates a block to store `layout`, and initializes it with the necessary metadata for the GC to drop it later.
    unsafe fn raw_allocate_with_drop(&self, layout: Layout, drop_in_place: unsafe fn(*mut ())) -> Result<NonNull<[u8]>, GCAllocatorError> {
        let (block, data) = unsafe { self.raw_allocate(layout)? };
        
        // SAFETY: dropper field is in bounds of the allocation, afaik this is similar to using `ptr::offset`
        let drop_ptr = unsafe { &raw mut (*block.as_ptr()).drop_in_place };
        // SAFETY: its fine to write here
        unsafe { drop_ptr.write(Some(drop_in_place)) };
        
        Ok(data)
    }
    
    pub fn allocate_for_type<T: Send>(&self) -> Result<NonNull<MaybeUninit<T>>, GCAllocatorError> {
        // TODO: support allocating dynamically sized types
        #[allow(unsafe_op_in_unsafe_fn)]
        unsafe fn dropper<T>(value: *mut ()) { std::ptr::drop_in_place(value as *mut T) }
        
        let type_layout = std::alloc::Layout::new::<T>();
        
        // using default is fine here. since `<*const T>::Metadata` is `()`, it literally doesnt matter
        let result = unsafe { self.raw_allocate_with_drop(type_layout, dropper::<T>) }?;
        
        // sanity check
        // SAFETY: length of slice is initialized, and whole slice fits in `isize`
        assert!(unsafe { std::mem::size_of_val_raw(result.as_ptr()) } >= std::mem::size_of::<T>());
        
        // truncate `result_block` to only have the requested size
        let result: NonNull<[u8]> = NonNull::from_raw_parts(result.cast(), type_layout.size());
        
        Ok(result.cast::<MaybeUninit<T>>())
    }
    
    /// Return whether or not the GC manages a given piece of data.
    pub fn contains<T: ?Sized>(&self, value: *const T) -> bool {
        self.memory_source.contains(value as *const ())
    }
    
    /// print out the bytes of the heap
    unsafe fn debug_heap(&self, _lock: &HeapLock) {
        let (mut address, length) = self.memory_source.raw_heap_data().to_raw_parts();
        let length = std::cmp::min(length, 1024);
        let end = unsafe { address.byte_add(length) };
        use std::io::Write;
        let mut lock = std::io::stdout().lock();
        while address < end {
            write!(lock, "[{:016x?}] ", address).unwrap();
            for i in 0..16 {
                write!(lock, "{:02x} ", unsafe { address.cast::<u8>().add(i).read() }).unwrap();
                if i == 7 {
                    write!(lock, "| ").unwrap();
                }
            }
            writeln!(lock).unwrap();
            address = unsafe { address.byte_add(16) };
        }
    }
}

unsafe impl<M> Allocator for GCAllocator<M> where M: Sync + MemorySource {
    /// NOTE: calling this directly will not initialize a destructor to run!!!
    /// (But usually the allocator API is only called in contexts where the destructor will run otherwise, so its whatever)
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, std::alloc::AllocError> {
        if layout.size() == 0 {
            return Err(std::alloc::AllocError) // pls no ZSTs thx
        }
        let (_header, data) = unsafe { self.raw_allocate(layout).map_err(|_e| std::alloc::AllocError) }?;
        Ok(data)
    }
    
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        // if we get here, we can add `ptr` to the free list, since whatever data has already been dropped.
        println!("TODO: deallocate (ptr={ptr:x?}, layout={layout:?}")
    }
}

#[cfg(target_os="windows")]
pub static GC_ALLOCATOR: LazyLock<GCAllocator<WindowsMemorySource>> = LazyLock::new(|| GCAllocator::new(&super::os_dependent::windows::mem_source::WIN_ALLOCATOR));
