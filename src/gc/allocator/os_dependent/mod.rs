
#[cfg(target_os="windows")]
pub mod windows;

use std::ptr::NonNull;
use std::sync::LazyLock;

#[cfg(target_os="windows")]
pub use windows::{StopAllThreads, mem_source::WindowsMemorySource};

/// shamelessly yoinked from https://github.com/ezrosent/allocators-rs/blob/master/elfmalloc/src/sources.rs
/// bc it is a very good abstraction
pub trait MemorySource {
    fn page_size(&self) -> usize;
    fn grow_by(&self, num_pages: usize) -> Option<NonNull<[u8]>>;
    unsafe fn shrink_by(&self, num_pages: usize);
    fn contains(&self, ptr: *const ()) -> bool;
    fn raw_heap_data(&self) -> NonNull<[u8]>;
}


#[cfg(target_os="windows")]
pub(super) type MemorySourceImpl = WindowsMemorySource;

pub(super) static MEMORY_SOURCE: &LazyLock<MemorySourceImpl> = if cfg!(windows) {
    &windows::mem_source::WIN_ALLOCATOR
} else {
    panic!("Other OS's memory sources")
};


