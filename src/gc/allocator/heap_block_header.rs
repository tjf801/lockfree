use std::alloc::Layout;
use std::ptr::NonNull;



pub(super) type HeaderFlag = usize;
pub(super) const HEADERFLAG_NONE: HeaderFlag = 0x00;
/// whether the heap block is allocated
/// 
/// TODO: also using `self.next == None` for this, can this be removed?
/// if so, what is the "end of list" sentinel value?
pub(super) const HEADERFLAG_ALLOCATED: HeaderFlag = 0x01;
/// Current heap block is marked as unreferenced, and can be dropped
pub(super) const HEADERFLAG_MARKED: HeaderFlag = 0x02;
/// Current heap block has been dropped and can be deallocated
pub(super) const HEADERFLAG_DROPPED: HeaderFlag = 0x04;
/// Current heap block is marked in the "unknown" category

/// NOTE: this struct must be followed by `self.size` contiguous bytes after it in memory.
#[repr(C, align(16))]
pub(super) struct GCHeapBlockHeader {
    pub(super) next: Option<NonNull<GCHeapBlockHeader>>,
    pub(super) size: usize,
    flags: HeaderFlag,
    pub(super) drop_in_place: Option<unsafe fn(*mut ())>,
}

impl GCHeapBlockHeader {
    /// Checks if the block is allocated.
    pub(super) fn is_allocated(&self) -> bool {
        self.flags & HEADERFLAG_ALLOCATED != 0 && self.next == None
    }
    
    /// Marks this block as allocated.
    /// 
    /// This is done by setting the appropriate flag, and setting the `next` pointer to null.
    pub(super) fn mark_allocated(&mut self) {
        assert!(!self.is_allocated(), "Block at {:016x?} was already allocated", self as *const _);
        self.flags |= HEADERFLAG_ALLOCATED;
        self.next = None; // if its allocated, its obviously not in the free list anymore
    }
    
    /// Gets the data associated with this value.
    /// 
    /// The returned pointer is directly after `self` in memory, and has length `self.length`.
    pub(super) fn data(&self) -> NonNull<[u8]> {
        let ptr = unsafe { NonNull::from(self).cast::<()>().byte_add(size_of::<Self>()) };
        let len = self.size;
        NonNull::from_raw_parts(ptr, len)
    }
    
    pub(super) fn next_mut(&mut self) -> Option<&mut Self> {
        self.next.map(|ptr| {
            // SAFETY: we have exclusive access to `self`..? (TODO?)
            unsafe { &mut *(ptr.as_ptr()) }
        })
    }
    
    /// Whether this block can trivially allocate for a given layout.
    pub(super) fn can_allocate(&self, layout: Layout) -> bool {
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
    pub(super) fn truncate_and_split(&mut self, num_bytes: usize) -> Result<NonNull<Self>, ()> {
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
    
    pub(super) unsafe fn write(self: *mut Self, next: Option<NonNull<Self>>, size: usize, flags: HeaderFlag, drop_in_place: Option<unsafe fn(*mut ())>) {
        unsafe {
            (&raw mut (*self).next).write(next);
            (&raw mut (*self).size).write(size);
            (&raw mut (*self).flags).write(flags);
            (&raw mut (*self).drop_in_place).write(drop_in_place);
        }
    }
    
    pub(super) unsafe fn write_new(self: *mut Self, next: Option<NonNull<Self>>, size: usize) {
        unsafe {
            (&raw mut (*self).next).write(next);
            (&raw mut (*self).size).write(size);
            (&raw mut (*self).flags).write(HEADERFLAG_NONE);
            (&raw mut (*self).drop_in_place).write(None);
        }
    }
}
