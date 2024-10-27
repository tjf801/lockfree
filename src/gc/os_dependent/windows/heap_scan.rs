// TODO: heap scan using
// https://learn.microsoft.com/en-us/windows/win32/api/heapapi/nf-heapapi-heaplock
// https://learn.microsoft.com/en-us/windows/win32/api/heapapi/nf-heapapi-heapwalk

use std::ptr::NonNull;


#[repr(transparent)]
pub struct WinHeap(NonNull<core::ffi::c_void>);

impl WinHeap {
    pub fn new() -> Result<Self, u32> {
        use windows_sys::Win32::System::Memory::GetProcessHeap;
        use windows_sys::Win32::Foundation::GetLastError;
        
        match NonNull::new(unsafe { GetProcessHeap() }) {
            None => {
                // TODO: better errors?
                Err(unsafe { GetLastError() })
            }
            Some(inner) => Ok(WinHeap(inner)),
        }
    }
    
    pub unsafe fn from_handle(handle: windows_sys::Win32::Foundation::HANDLE) -> Option<Self> {
        // TODO: what are the requirements for this function? obviously passing in some random value could probably be bad but idk
        Some(Self(NonNull::new(handle)?))
    }
    
    pub fn handle(&self) -> windows_sys::Win32::Foundation::HANDLE {
        self.0.as_ptr()
    }
    
    pub fn lock(&self) -> Result<WinHeapLock<'_>, u32> {
        // TODO: make better errors than a u32 error code?
        WinHeapLock::new(self)
    }
}

impl Drop for WinHeap {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::{CloseHandle, GetLastError};
        if unsafe { CloseHandle(self.handle()) } == 0 {
            let _err = unsafe { GetLastError() };
            // TODO: handle error..?
        }
    }
}


#[repr(C)]
#[derive(Clone, Copy)]
pub struct WinHeapEntry(windows_sys::Win32::System::Memory::PROCESS_HEAP_ENTRY);


#[derive(Debug, Clone, Copy)]
pub struct WinHeapRegionInfo {
    /// Pointer to the first valid memory block in this heap region.
    pub first_block: *const core::ffi::c_void,
    /// Pointer to the first invalid memory block in this heap region.
    pub last_block: *const core::ffi::c_void,
    /// Number of bytes in the heap region that are currently committed as free memory blocks, busy memory blocks, or heap control structures.
    /// 
    /// This is an optional field that is set to zero if the number of committed bytes is not available.
    pub committed_size: usize,
    /// Number of bytes in the heap region that are currently uncommitted.
    /// 
    /// This is an optional field that is set to zero if the number of uncommitted bytes is not available.
    pub uncommitted_size: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct WinHeapBlockInfo {
    /// Handle to the allocated, moveable memory block.
    pub mem_handle: *mut core::ffi::c_void,
}

/// https://learn.microsoft.com/en-us/windows/win32/api/minwinbase/ns-minwinbase-process_heap_entry
impl WinHeapEntry {
    /// The heap element is an allocated block.
    /// 
    /// If `HEAP_ENTRY_MOVEABLE` is also specified, the `Block` structure becomes
    /// valid. The `hMem` member of the `Block` structure contains a handle to the
    /// allocated, moveable memory block.
    const HEAP_ENTRY_BUSY: u16 = windows_sys::Win32::System::SystemServices::PROCESS_HEAP_ENTRY_BUSY as u16;
    /// This value must be used with PROCESS_HEAP_ENTRY_BUSY, indicating that the heap element is an allocated block.
    /// 
    /// If this flag is valid and set, the block was allocated with the
    /// `GMEM_DDESHARE` flag. For a discussion of the `GMEM_DDESHARE` flag, see
    /// `GlobalAlloc` .
    const HEAP_ENTRY_DDESHARE: u16 = windows_sys::Win32::System::SystemServices::PROCESS_HEAP_ENTRY_DDESHARE as u16;
    /// This value must be used with `PROCESS_HEAP_ENTRY_BUSY`, indicating that the heap element is an allocated block.
    /// 
    /// The block was allocated with `LMEM_MOVEABLE` or `GMEM_MOVEABLE`, and the
    /// `Block` structure becomes valid. The `hMem` member of the `Block`
    /// structure contains a handle to the allocated, moveable memory block.
    const HEAP_ENTRY_MOVEABLE: u16 = windows_sys::Win32::System::SystemServices::PROCESS_HEAP_ENTRY_MOVEABLE as u16;
    /// The heap element is located at the beginning of a region of contiguous virtual memory in use by the heap.
    /// 
    /// The `lpData` member of the structure points to the first virtual address
    /// used by the region; the `cbData` member specifies the total size, in
    /// bytes, of the address space that is reserved for this region; and the
    /// `cbOverhead` member specifies the size, in bytes, of the heap control
    /// structures that describe the region.
    /// 
    /// The `Region` structure becomes valid. The `dwCommittedSize`,
    /// `dwUnCommittedSize`, `lpFirstBlock`, and `lpLastBlock` members of the
    /// structure contain additional information about the region.
    const HEAP_REGION: u16 = windows_sys::Win32::System::SystemServices::PROCESS_HEAP_REGION as u16;
    /// The heap element is located in a range of uncommitted memory within the heap region.
    /// 
    /// The `lpData` member points to the beginning of the range of uncommitted
    /// memory; the `cbData` member specifies the size, in bytes, of the range
    /// of uncommitted memory; and the `cbOverhead` member specifies the size,
    /// in bytes, of the control structures that describe this uncommitted range.
    const HEAP_UNCOMMITTED_RANGE: u16 = windows_sys::Win32::System::SystemServices::PROCESS_HEAP_UNCOMMITTED_RANGE as u16;
    
    fn new(raw_entry: windows_sys::Win32::System::Memory::PROCESS_HEAP_ENTRY) -> Self {
        Self(raw_entry)
    }
    
    
    pub fn is_allocated(&self) -> bool {
        self.0.wFlags & Self::HEAP_ENTRY_BUSY != 0
    }
    
    pub fn is_uncommitted_range(&self) -> bool {
        self.0.wFlags & Self::HEAP_UNCOMMITTED_RANGE != 0
    }
    
    pub fn is_region(&self) -> bool {
        self.0.wFlags & Self::HEAP_REGION != 0
    }
    
    pub fn is_moveable(&self) -> bool {
        self.is_allocated() && (self.0.wFlags & Self::HEAP_ENTRY_MOVEABLE != 0)
    }
    
    
    pub fn region_info(&self) -> Option<WinHeapRegionInfo> {
        if self.is_region() {
            // SAFETY: see windows documentation on PROCESS_HEAP_REGION
            Some(unsafe {WinHeapRegionInfo {
                first_block: self.0.Anonymous.Region.lpFirstBlock,
                last_block: self.0.Anonymous.Region.lpLastBlock,
                committed_size: self.0.Anonymous.Region.dwCommittedSize as usize,
                uncommitted_size: self.0.Anonymous.Region.dwUnCommittedSize as usize,
            }})
        } else {
            None
        }
    }
    
    pub fn block_info(&self) -> Option<WinHeapBlockInfo> {
        if self.is_moveable() {
            // SAFETY: see docs on PROCESS_HEAP_ENTRY_MOVEABLE
            Some(unsafe {WinHeapBlockInfo {
                mem_handle: self.0.Anonymous.Block.hMem
            }})
        } else {
            None
        }
    }
    
    
    pub fn data(&self) -> *const core::ffi::c_void {
        self.0.lpData
    }
    
    pub fn size(&self) -> usize {
        self.0.cbData as usize + self.0.cbOverhead as usize
    }
    
    pub fn data_size(&self) -> usize {
        self.0.cbData as usize
    }
}

#[must_use = "if unused the heap will immediately unlock"]
pub struct WinHeapLock<'lock>(&'lock WinHeap);

impl<'lock> WinHeapLock<'lock> {
    fn new(heap: &'lock WinHeap) -> Result<Self, u32> {
        use windows_sys::Win32::System::Memory::HeapLock;
        use windows_sys::Win32::Foundation::GetLastError;
        
        // WHY DOES THIS BLOCK I DONT WANT IT TO BLOCK 🤬😡😠
        // update: apparently this is just a syscall and windows literally does
        // not expose a non-blocking `HeapLock` equivalent, and i am not smart
        // enough to go digging around in the windows kernel to figure out how
        // to make one in user land, or even just CHECK if a heap is locked
        if unsafe { HeapLock(heap.handle()) } == 0 {
            let err = unsafe { GetLastError() };
            return Err(err);
        }
        
        Ok(Self(heap))
    }
    
    pub fn unlock(self) {
        drop(self);
    }
    
    pub fn walk(&self) -> impl Iterator<Item=WinHeapEntry> {
        use windows_sys::Win32::System::Memory::HeapWalk;
        use windows_sys::Win32::Foundation::{ERROR_NO_MORE_ITEMS, GetLastError};
        
        gen {
            let mut entry = windows_sys::Win32::System::Memory::PROCESS_HEAP_ENTRY {
                lpData: std::ptr::null_mut(),
                cbData: 0,
                cbOverhead: 0,
                iRegionIndex: 0,
                wFlags: 0,
                Anonymous: windows_sys::Win32::System::Memory::PROCESS_HEAP_ENTRY_0 {
                    Block: windows_sys::Win32::System::Memory::PROCESS_HEAP_ENTRY_0_0 {
                        hMem: std::ptr::null_mut(),
                        dwReserved: [0, 0, 0],
                    }
                }
            };
            
            loop {
                if unsafe { HeapWalk(self.0.handle(), &raw mut entry) } == 0 {
                    let err = unsafe { GetLastError() };
                    if err == ERROR_NO_MORE_ITEMS {
                        return
                    }
                    panic!("Error in HeapWalk: (code {err:x})");
                }
                
                yield WinHeapEntry::new(entry);
            }
        }
    }
}

impl Drop for WinHeapLock<'_> {
    fn drop(&mut self) {
        use windows_sys::Win32::System::Memory::HeapUnlock;
        use windows_sys::Win32::Foundation::GetLastError;
        if unsafe { HeapUnlock(self.0.handle()) } == 0 {
            panic!("failed to unlock heap (error {:x})", unsafe { GetLastError() })
        }
    }
}

/// If [`HeapLock`] succeeds, the calling thread owns the heap lock. Only the
/// calling thread will be able to allocate or release memory from the heap. The
/// execution of any other thread of the calling process will be blocked if that
/// thread attempts to allocate or release memory from the heap. Such threads
/// will remain blocked until the thread that owns the heap lock calls the
/// HeapUnlock function.
/// 
/// [`HeapLock`] https://learn.microsoft.com/en-us/windows/win32/api/heapapi/nf-heapapi-heaplock
impl !Send for WinHeapLock<'_> {}

pub fn get_all_heaps() -> impl Iterator<Item=WinHeap> {
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::Memory::GetProcessHeaps;
    
    let num_heaps = unsafe { GetProcessHeaps(0, std::ptr::null_mut()) };
    
    let mut heap_handles = Box::<[HANDLE]>::new_uninit_slice(num_heaps as usize);
    let ptr = heap_handles.get_mut(0).unwrap().as_mut_ptr();
    
    let rv = unsafe { GetProcessHeaps(num_heaps, ptr) };
    assert_eq!(rv, num_heaps);
    
    // SAFETY: GetProcessHeaps initializes the data in heap_handles
    let heap_handles = unsafe { heap_handles.assume_init() };
    
    heap_handles.into_iter().map(|h| unsafe { WinHeap::from_handle(h).unwrap_unchecked() })
}

// #[test]
// fn scan_process_heaps() {
//     for heap in get_all_heaps() {
//         println!("HEAP:");
//         for block in heap.lock().unwrap().walk() {
//             if !block.is_region() {
//                 if !block.is_uncommitted_range() {
//                     println!("  {:x?}[{:04}/{:04}] ({})", block.0.lpData, block.data_size(), block.size(), block.is_allocated())
//                 } else {
//                     println!("UNCOMMITTED\n{:x?}[{}/{}]", block.data(), block.data_size(), block.size())
//                 }
//             } else {
//                 let info = block.region_info().unwrap();
//                 println!("REGION\n{:x?} to {:x?} [{}]", info.first_block, info.last_block, info.committed_size);
//             }
//         }
//     }
//     info!("heap_scan::test: Successfully enumerated all heaps");
// }
