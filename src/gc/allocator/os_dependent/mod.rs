use std::ptr::NonNull;
use std::sync::LazyLock;

#[cfg(target_os="windows")]
mod windows;

pub use windows::get_writable_segments;

/// shamelessly yoinked from https://github.com/ezrosent/allocators-rs/blob/master/elfmalloc/src/sources.rs
/// bc it is a very good abstraction
pub trait MemorySource {
    /// The amount of bytes in a page.
    fn page_size(&self) -> usize;
    
    /// Get `num_pages * self.page_size()` bytes of memory.
    /// 
    /// The memory is not necessarily initialized.
    fn grow_by(&self, num_pages: usize) -> Option<NonNull<[u8]>>;
    
    /// Removes pages from the pool of allocated memory.
    unsafe fn shrink_by(&self, num_pages: usize);
    
    /// Whether the given pointer points into the memory pool.
    fn contains(&self, ptr: *const ()) -> bool;
    
    /// A pointer into the entire pool of committed memory.
    fn raw_data(&self) -> NonNull<[u8]>;
}

#[cfg(target_os="windows")]
pub use windows::mem_source::WindowsMemorySource;

#[cfg(target_os="windows")]
pub(super) type MemorySourceImpl = WindowsMemorySource;

pub(super) static MEMORY_SOURCE: &LazyLock<MemorySourceImpl> = if cfg!(windows) {
    &windows::mem_source::WIN_ALLOCATOR
} else if cfg!(unix) {
    panic!("TODO: posix api")
} else {
    panic!("Other OSes are not supported")
};


#[cfg(target_os="windows")]
pub use windows::{get_all_threads, get_thread_stack_bounds, StopAllThreads, heap_scan};


