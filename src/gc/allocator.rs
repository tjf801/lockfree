use std::alloc::{AllocError, Allocator, Layout};
use std::mem::MaybeUninit;
use std::ptr::NonNull;
use std::sync::{Condvar, LazyLock, Mutex, RwLock};

mod collector;
mod heap_block_header;
mod tl_allocator;
mod os_dependent;

use collector::{DEALLOCATED_CHANNEL, gc_main};
use heap_block_header::GCHeapBlockHeader;
use os_dependent::{MemorySource, MemorySourceImpl, MEMORY_SOURCE};
use thread_local::ThreadLocal;
use tl_allocator::TLAllocator;


static THREAD_LOCAL_ALLOCATORS: RwLock<ThreadLocal<TLAllocator<MemorySourceImpl>>> = RwLock::new(ThreadLocal::new());

static GC_CYCLE_NUMBER: Mutex<usize> = Mutex::new(0);
static GC_CYCLE_SIGNAL: Condvar = Condvar::new();

/// Returns the GC heap block that a given pointer points into.
fn get_block(ptr: *const ()) -> Option<NonNull<GCHeapBlockHeader>> {
    if !MEMORY_SOURCE.contains(ptr) {
        return None
    }
    
    let (block_ptr, heap_size) = MEMORY_SOURCE.raw_data().to_raw_parts();
    let end = unsafe { block_ptr.byte_add(heap_size).cast() };
    let mut block_ptr = block_ptr.cast::<GCHeapBlockHeader>();
    
    while block_ptr < end {
        if ptr > block_ptr.as_ptr().cast() { return Some(block_ptr) }
        block_ptr = unsafe { block_ptr.as_ref() }.next();
    }
    if block_ptr != end {
        error!("Heap corruption detected (expected to end at {end:016x?}, got {block_ptr:016x?})")
    }
    
    None
}


#[non_exhaustive]
#[derive(Debug, Clone, Copy)]
pub enum GCAllocatorError {
    ZeroSized,
    BadAlignment,
    OutOfMemory,
}


pub struct GCAllocator;

impl GCAllocator {
    /// Puts the value into the GCed heap.
    pub fn allocate_for_value<T: Send>(&self, value: T) -> Result<NonNull<T>, (GCAllocatorError, T)> {
        let tl_reader = THREAD_LOCAL_ALLOCATORS.read().unwrap();
        let allocator = match tl_reader.get_or_try(|| TLAllocator::try_new(MEMORY_SOURCE)) {
            Ok(a) => a,
            Err(e) => return Err((e, value))
        };
        
        match allocator.allocate_for_value(value) {
            // If the GC was out of memory, then we wait for a GC cycle to free up memory before trying again.
            Err((GCAllocatorError::OutOfMemory, value)) => {
                warn!("Got an `OutOfMemory` error on allocation, trying again after GC...");
                self.wait_for_gc();
                // If the GC is *still* out of memory, just give up.
                allocator.allocate_for_value(value)
            },
            // Otherwise, just forward whatever we got
            r => r
        }
    }
    
    /// Return whether or not a pointer points into the GC heap.
    pub fn contains<T: ?Sized>(&self, value: *const T) -> bool {
        MEMORY_SOURCE.contains(value as *const ())
    }
    
    /// Blocks until the GC has done a full collection cycle.
    pub fn wait_for_gc(&self) {
        debug!("Waiting for a GC cycle");
        
        let mut guard = GC_CYCLE_NUMBER.lock().unwrap();
        let cycle = *guard;
        
        // block until the cycle number has incremented
        while cycle == *guard {
            guard = GC_CYCLE_SIGNAL.wait(guard).unwrap();
        }
    }
}

unsafe impl Allocator for GCAllocator {
    /// NOTE: Do not use this method directly if you want your stuff to be automatically dropped!
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        if layout.size() == 0 {
            return Err(std::alloc::AllocError) // pls no ZSTs thx
        }
        
        let tl_reader = THREAD_LOCAL_ALLOCATORS.read().unwrap();
        let allocator = tl_reader.get_or_try(|| TLAllocator::try_new(MEMORY_SOURCE)).map_err(|_| AllocError)?;
        
        let (_header, block) = allocator.raw_allocate(layout).map_err(|_| AllocError)?;
        
        Ok(block)
    }
    
    /// Frees a piece of memory in the GC heap referenced by `ptr`.
    /// 
    /// This does **not** run any destructor associated with the type in the heap.
    /// 
    /// # Safety
    /// (taken from [`Allocator::deallocate`])
    /// * `ptr` must denote a block of memory [*currently allocated*] via this allocator
    /// * `layout` must [*fit*] that block of memory
    /// * `ptr` cannot have any dangling references into it.
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        // sanity check
        assert!(ptr.is_aligned_to(layout.align()));
        
        let data: NonNull<[u8]> = NonNull::from_raw_parts(ptr, layout.size());
        
        // If we got here, we can't run the destructor again
        // TODO: should we just `unwrap_unchecked` here? this is a pretty reasonable precondition
        let block = get_block(ptr.as_ptr() as _).expect("Freed pointer should point into the GC heap").as_ptr();
        unsafe { (*block).drop_thunk = None };
        
        DEALLOCATED_CHANNEL.wait().send(data.into()).expect("The GC thread shouldn't ever exit");
    }
}

pub static GC_ALLOCATOR: LazyLock<GCAllocator> = LazyLock::new(|| {
    use simplelog::*;
    use std::fs::File;
    
    // initialize logging
    CombinedLogger::init(
        vec![
            TermLogger::new(LevelFilter::Warn, Config::default(), TerminalMode::Mixed, ColorChoice::Auto),
            WriteLogger::new(LevelFilter::Debug, Config::default(), File::create("gc_debug.log").unwrap()),
        ]
    ).unwrap();
    
    // start collector thread
    std::thread::spawn(gc_main);
    GCAllocator
});
