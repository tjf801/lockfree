use std::ptr::NonNull;
use std::sync::{LazyLock, RwLock};

use windows_sys::Win32::Foundation::GetLastError;
use windows_sys::Win32::System::Memory::{MEM_RESERVE, MEM_COMMIT, PAGE_READWRITE, VirtualAlloc};

struct MemSizes {
    /// The current size of the heap
    length: usize,
    /// the "capacity" of the heap
    committed: usize,
}

pub struct WindowsMemorySource {
    data: *mut (),
    /// maximum allowed capacity of the heap
    reserved: usize, // constant
    sizes: RwLock<MemSizes>,
}

// SAFETY: `data` is the only thing not `Send`/`Sync` here, but we dont actually ever change it
unsafe impl Send for WindowsMemorySource {}
unsafe impl Sync for WindowsMemorySource {}

impl WindowsMemorySource {
    /// the page size of the system
    const PAGE_SIZE: usize = 0x1000;
    
    // TODO: should there be equivalents to `-Xms` and `-Xmx`? or some better way to configure this
    
    /// default size is 32MiB
    const FIRST_COMMIT_SIZE: usize = 0x2000000;
    /// default max size is 2GiB
    const DEFAULT_MAX_SIZE: usize = 0x20000000000;
    
    fn new(max_size: usize) -> Self {
        // Reserve maximum capacity
        let base_ptr = unsafe { VirtualAlloc(std::ptr::null(), max_size, MEM_RESERVE, PAGE_READWRITE) } as *mut ();
        if base_ptr.is_null() {
            let err = unsafe { GetLastError() };
            panic!("First reserve failed with code {:x}", err);
        }
        
        // Commit first page
        // TODO: make Self::FIRST_PAGE_SIZE a parameter ?
        let page = unsafe { VirtualAlloc(base_ptr as _, Self::FIRST_COMMIT_SIZE, MEM_COMMIT, PAGE_READWRITE) } as *mut ();
        if page.is_null() {
            let err = unsafe { GetLastError() };
            panic!("First commit failed with code {:x}", err);
        }
        
        assert_eq!(page, base_ptr);
        
        Self {
            data: base_ptr,
            reserved: max_size,
            sizes: RwLock::new(MemSizes {
                length: 0,
                committed: Self::FIRST_COMMIT_SIZE
            })
        }
    }
}

impl super::super::MemorySource for WindowsMemorySource {
    fn page_size(&self) -> usize {
        Self::PAGE_SIZE
    }
    
    fn grow_by(&self, num_pages: usize) -> Option<NonNull<[u8]>> {
        // TODO: improve readability at some point
        let MemSizes { length, committed } = &mut *self.sizes.write().ok()?; // panic safety: we don't already hold the write lock
        let old_length = *length;
        *length += num_pages * self.page_size();
        
        // not enough memory for the requested allocation
        if *length > self.reserved {
            *length = old_length;
            return None;
        }
        
        while committed < length {
            // place to allocate more memory from
            let new_base = self.data.wrapping_byte_offset(*committed as isize);
            
            // allocate more memory, growing geometrically
            let rv = unsafe { VirtualAlloc(new_base as _, *committed, MEM_COMMIT, PAGE_READWRITE) } as *mut ();
            if rv.is_null() {
                let err = unsafe { GetLastError() };
                error!("Commit failed with code {:x}", err);
                return None;
            }
            
            // amount of committed memory just grew by `*committed` bytes
            *committed += *committed;
        }
        
        // SAFETY: entire address space in [`data`, `data+length`) is valid, and old_length ≤ length
        let ptr = unsafe { self.data.byte_offset(old_length as isize) };
        let len = num_pages * self.page_size();
        
        Some(NonNull::<[u8]>::from_raw_parts(NonNull::new(ptr)?, len))
    }
    
    unsafe fn shrink_by(&self, num_pages: usize) {
        let MemSizes { length, .. } = &mut *self.sizes.write().expect("Should never panic while holding lock");
        *length -= num_pages * self.page_size();
    }
    
    fn contains(&self, ptr: *const ()) -> bool {
        let min = self.data.addr();
        let max = min + self.sizes.read().unwrap().length;
        let value = ptr.addr();
        min <= value && value < max
    }
    
    fn raw_heap_data(&self) -> NonNull<[u8]> {
        NonNull::from_raw_parts(
            NonNull::new(self.data).expect("heap pointer is never null"),
            self.sizes.read().unwrap().length
        )
    }
}

/// Default maximum memory: 2GiB
pub static WIN_ALLOCATOR: LazyLock<WindowsMemorySource> = LazyLock::new(|| WindowsMemorySource::new(WindowsMemorySource::DEFAULT_MAX_SIZE));

#[cfg(test)]
mod tests {
    
}

