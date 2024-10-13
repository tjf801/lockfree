use std::ptr::NonNull;
use std::alloc::{Allocator, Layout};
use std::sync::{LazyLock, OnceLock};

pub struct GCAllocator {
    // TODO: black/gray/white allocations
    
}

struct GCAllocHeader {
    next: Option<NonNull<GCAllocHeader>>,
    size: usize,
}

impl GCAllocator {
    fn new() -> Self {
        Self {}
    }
    
    fn block_layout_from_layout(layout: Layout) -> (Layout, usize) {
        Layout::new::<GCAllocHeader>().extend(layout).unwrap()
    }
    
    /// Return whether or not the GC manages a given piece of data.
    pub fn manages_ptr<T: ?Sized>(&self, value: *const T) -> bool {
        todo!()
    }
}

unsafe impl Allocator for GCAllocator {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, std::alloc::AllocError> {
        let (real_layout, idx) = GCAllocator::block_layout_from_layout(layout);
        todo!()
    }
    
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        // if we get here, we can add `ptr` to the "definitely able to free" list.
        todo!()
    }
}

pub static GC_ALLOCATOR: LazyLock<GCAllocator> = LazyLock::new(GCAllocator::new);
