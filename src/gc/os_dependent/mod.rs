
#[cfg(target_os="windows")]
pub mod windows;

use std::ptr::NonNull;

#[cfg(target_os="windows")]
pub use windows::{get_all_thread_stack_bounds, start_the_world, stop_the_world, mem_source::WindowsMemorySource};

/// shamelessly yoinked from https://github.com/ezrosent/allocators-rs/blob/master/elfmalloc/src/sources.rs
/// bc it is a very good abstraction
pub trait MemorySource {
    fn page_size(&self) -> usize;
    fn grow_by(&self, num_pages: usize) -> Option<NonNull<[u8]>>;
    unsafe fn shrink_by(&self, num_pages: usize);
    fn contains(&self, ptr: *const ()) -> bool;
}


