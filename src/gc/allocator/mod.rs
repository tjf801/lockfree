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
const HEADERFLAG_ALLOCATED: HeaderFlag = 0x01;
const HEADERFLAG_OK_TO_DROP: HeaderFlag = 0x02;
const HEADERFLAG_DROPPED: HeaderFlag = 0x04;

#[repr(C, align(16))]
struct GCObjectHeader {
    next: Option<NonNull<GCObjectHeader>>,
    size: usize,
    flags: HeaderFlag,
    drop_in_place: Option<unsafe fn(*mut ())>,
}

impl GCObjectHeader {
    fn is_allocated(&self) -> bool {
        self.flags & HEADERFLAG_ALLOCATED != 0
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
}


// TODO: tri-color allocations
pub struct GCAllocator<M: 'static + Sync + MemorySource> {
    memory_source: &'static M,
    gc_thread: std::thread::JoinHandle<()>,
    heap_lock: AtomicU8,
    heap_freelist_head: UnsafeCell<Option<NonNull<GCObjectHeader>>>,
}

// TODO: actually make sure this is thread-safe lmao
unsafe impl<M: Sync + MemorySource> Send for GCAllocator<M> {}
unsafe impl<M: Sync + MemorySource> Sync for GCAllocator<M> {}

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
    
    fn lock_heap(&self) {
        // TODO: use an actually good lock, either in `std::sys` or some other library
        loop {
            // wait until we load a 0
            while self.heap_lock.load(Ordering::Relaxed) != 0 { std::hint::spin_loop() }
            match self.heap_lock.compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed) {
                    Ok(_) => { break },
                    Err(_) => { } // someone else got here before us
            }
        }
    }
    
    fn unlock_heap(&self) {
        // release the lock
        match self.heap_lock.compare_exchange(1, 0, Ordering::Release, Ordering::Relaxed) {
            Err(_) => panic!("We set it to 1, no other thread shouldve messed with the lock"),
            Ok(_) => {}
        }
    }
    
    // Expands the heap by at least the given number of bytes, and the new block to the free list
    fn expand_heap(&self, num_bytes: usize) -> Result<NonNull<GCObjectHeader>, GCAllocatorError> {
        // assert that the heap is locked
        // TODO: make sure *we* actually hold the lock
        assert_eq!(self.heap_lock.load(Ordering::Relaxed), 1);
        
        // allocate some space
        let page_size = self.memory_source.page_size();
        let num_pages = num_bytes.div_ceil(page_size);
        let new_ptr = self.memory_source.grow_by(num_pages).ok_or(GCAllocatorError::OutOfMemory)?;
        
        // update the new heap block
        let new_block = new_ptr.cast::<GCObjectHeader>();
        
        // SAFETY: we own this data, its fine to write
        unsafe {
            let pointer = new_block.as_ptr();
            let block_len = new_ptr.len() - size_of::<GCObjectHeader>();
            (&raw mut (*pointer).next).write(None);
            (&raw mut (*pointer).size).write(block_len);
            (&raw mut (*pointer).flags).write(HEADERFLAG_NONE);
            (&raw mut (*pointer).drop_in_place).write(None);
        }
        
        Ok(new_ptr.cast())
    }
    
    unsafe fn raw_allocate(&self, layout: Layout) -> Result<(NonNull<GCObjectHeader>, NonNull<[u8]>), GCAllocatorError> {
        // TODO: support bigger alignments than 16
        if layout.align() > 16 {
            return Err(GCAllocatorError::AlignmentTooHigh)
        }
        
        self.lock_heap();
        
        // make sure we actually have some free memory lol
        // SAFETY: we hold the heap lock so this is fine
        if unsafe { (*self.heap_freelist_head.get()).is_none() } {
            // TODO: heap never unlocks if error here...
            let result = self.expand_heap(layout.size())?;
            // SAFETY: TODO
            unsafe { self.heap_freelist_head.get().write(Some(result)) }
        }
        
        let (previous_block, result_block) = {
            let mut previous_block: Option<NonNull<GCObjectHeader>> = None;
            let mut current_block = unsafe { *self.heap_freelist_head.get() }.expect("should have just set to Some()");
            loop {
                let current_block_ref = unsafe { current_block.as_ref() };
                assert!(!current_block_ref.is_allocated());
                if current_block_ref.can_allocate(layout) {
                    break (previous_block, current_block_ref);
                }
                
                match current_block_ref.next {
                    Some(next_block) => {
                        previous_block = Some(current_block);
                        current_block = next_block;
                    }
                    None => {
                        // TODO: grow memory
                        todo!()
                    }
                }
            }
        };
        
        match previous_block {
            None => {
                let first = unsafe { *self.heap_freelist_head.get() };
                // first entry in the heap list
                assert_eq!(first.unwrap().as_ptr() as *const _, result_block as *const GCObjectHeader);
            }
            Some(previous_block) => unsafe {
                // pop out `result_block` from the linked list
                (*previous_block.as_ptr()).next = result_block.next
            }
        }
        
        self.unlock_heap();
        
        Ok((result_block.into(), result_block.data()))
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
        assert_eq!(unsafe { std::mem::size_of_val_raw(result.as_ptr()) }, std::mem::size_of::<T>());
        
        Ok(result.cast::<MaybeUninit<T>>())
    }
    
    /// Return whether or not the GC manages a given piece of data.
    pub fn contains<T: ?Sized>(&self, value: *const T) -> bool {
        self.memory_source.contains(value as *const ())
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
