use std::mem::MaybeUninit;
use std::ptr::NonNull;
use std::alloc::{Allocator, Layout};
use std::sync::LazyLock;

use super::os_dependent::windows::mem_source::WindowsMemorySource;
use super::os_dependent::MemorySource;


// TODO: tri-color allocations
pub struct GCAllocator<M: 'static + Sync> {
    memory_source: &'static M,
    // heap_lock: std::sys::sync::Mutex,
}

// TODO: actually make this thread-safe lmao
unsafe impl<M: Sync> Send for GCAllocator<M> {}
unsafe impl<M: Sync> Sync for GCAllocator<M> {}

#[non_exhaustive]
#[derive(Debug, Clone, Copy)]
pub enum GCAllocatorError {
    ZeroSized,
    AlignmentTooHigh,
    OutOfMemory,
}

type HeaderFlag = usize;
const HEADERFLAG_ALLOCATED: HeaderFlag = 0x01;
const HEADERFLAG_OK_TO_DROP: HeaderFlag = 0x02;
const HEADERFLAG_DROPPED: HeaderFlag = 0x04;

#[repr(C, align(16))]
struct GCObjectHeader {
    next: Option<NonNull<GCObjectHeader>>,
    flags: HeaderFlag,
    drop_in_place: Option<unsafe fn(*mut ())>,
}

impl<M> GCAllocator<M> where M: Sync + MemorySource {
    fn new() -> Self {
        todo!()
    }
    
    unsafe fn raw_allocate(&self, layout: Layout) -> Result<(NonNull<GCObjectHeader>, NonNull<[u8]>), GCAllocatorError> {
        todo!()
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
        // if we get here, we can add `ptr` to the "definitely able to free" list.
        todo!()
    }
}

#[cfg(target_os="windows")]
pub static GC_ALLOCATOR: LazyLock<GCAllocator<WindowsMemorySource>> = LazyLock::new(GCAllocator::new);
