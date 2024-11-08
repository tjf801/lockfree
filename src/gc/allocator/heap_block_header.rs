use std::alloc::Layout;
use std::mem::MaybeUninit;
use std::ptr::NonNull;



pub(super) type HeaderFlag = usize;
pub(super) const HEADERFLAG_NONE: HeaderFlag = 0x00;
/// whether the heap block is allocated
/// 
/// TODO: also using `self.next == None` for this, can this be removed?
/// if so, what is the "end of list" sentinel value?
pub(super) const HEADERFLAG_ALLOCATED: HeaderFlag = 0x01;

/// NOTE: this struct must be followed by `self.size` contiguous bytes after it in memory.
#[repr(C, align(16))]
pub(super) struct GCHeapBlockHeader {
    pub(super) next_free: Option<NonNull<GCHeapBlockHeader>>,
    pub(super) size: usize,
    pub(super) flags: HeaderFlag,
    pub(super) drop_thunk: Option<unsafe fn(*mut ())>,
}

#[derive(Clone, Debug)]
pub(super) enum BlockFittingError {
    BlockTooSmall,
    CantFitNextBlock,
    NotEnoughAlignedRoom,
}

impl GCHeapBlockHeader {
    /// Checks if the block is allocated.
    pub(super) fn is_allocated(&self) -> bool {
        if self.flags & HEADERFLAG_ALLOCATED != 0 { assert!(self.next_free.is_none()) }
        self.flags & HEADERFLAG_ALLOCATED != 0
    }
    
    /// Marks this block as allocated.
    /// 
    /// This is done by setting the appropriate flag, and setting the `next` pointer to null.
    pub(super) fn set_allocated(&mut self) {
        if self.is_allocated() {
            error!("Block at {:016x?} was already allocated", self as *const _);
        }
        assert!(!self.is_allocated(), "Block at {:016x?} was already allocated", self as *const _);
        self.flags |= HEADERFLAG_ALLOCATED;
        self.next_free = None; // if its allocated, its obviously not in the free list anymore
    }
    
    /// Unmarks this block as deallocated.
    /// 
    /// This is done by setting the appropriate flag, and setting the `next` pointer to null.
    pub(super) fn set_free(&mut self, next: Option<NonNull<GCHeapBlockHeader>>) {
        if !self.is_allocated() {
            error!("Block at {:016x?} was already deallocated", self as *const _);
        }
        assert!(self.is_allocated(), "Block at {:016x?} was already deallocated", self as *const _);
        self.flags &= !HEADERFLAG_ALLOCATED;
        self.next_free = next;
    }
    
    /// Gets the data associated with this value.
    /// 
    /// The returned pointer is directly after `self` in memory, and has length `self.length`.
    /// 
    /// It's only safe to create a reference into this data if the block is not allocated.
    pub(super) fn data(&self) -> NonNull<[u8]> {
        let ptr = unsafe { NonNull::from(self).cast::<()>().byte_add(size_of::<Self>()) };
        let len = self.size;
        NonNull::from_raw_parts(ptr, len)
    }
    
    // The next free block, regardless of whether it is free or not
    pub(super) fn next(&self) -> NonNull<Self> {
        // SAFETY: this points to the end of this block
        unsafe { NonNull::from(self).byte_add(size_of_val(self) + self.size) }
    }
    
    pub(super) fn shrink_to_fit(&mut self, layout: Layout) -> Result<(&mut Self, usize), BlockFittingError> {
        assert!(!self.is_allocated());
        assert!(self.size >= align_of::<Self>());
        
        let (size, align) = (layout.size(), layout.align());
        let align = std::cmp::max(align, align_of::<Self>());
        
        let padded_size = size.next_multiple_of(align_of::<Self>());
        
        // trivially not able to hold layout
        if self.data().len() < padded_size {
            return Err(BlockFittingError::BlockTooSmall)
        }
        
        // block data is already aligned
        if self.data().is_aligned_to(align) {
            if self.data().len() > padded_size + size_of::<Self>() {
                // split off another block (of size > 0) at end
                
                let next_block_size = self.data().len() - padded_size - size_of::<Self>();
                assert!(next_block_size > 0); // sanity check
                let next_block = unsafe { self.data().byte_add(padded_size).cast::<MaybeUninit<Self>>().as_mut() };
                let next_block = next_block.write(GCHeapBlockHeader {
                    next_free: self.next_free,
                    flags: HEADERFLAG_NONE,
                    size: next_block_size,
                    drop_thunk: None
                });
                
                self.next_free = Some(next_block.into());
                self.size = padded_size;
                
                // this block is the one that fits the layout
                return Ok((self, size_of::<Self>()))
            }
            
            // no need to split off a block at the end, since theres no next block
            if self.next_free.is_none() && self.data().len() >= padded_size {
                return Ok((self, 0))
            }
            
            // cant fit next block data at the end
            return Err(BlockFittingError::CantFitNextBlock)
        }
        
        // NOTE: now we know that align is greater than align_of::<Self>()
        
        let next_aligned = self.data().cast::<()>().map_addr(|a| unsafe {
            std::num::NonZero::new((usize::from(a) + size_of::<Self>() + 1).next_multiple_of(align)).unwrap_unchecked()
        }).cast::<MaybeUninit<Self>>();
        let data_end = unsafe { self.data().cast::<()>().byte_add(self.data().len()) };
        
        if unsafe { next_aligned.byte_add(padded_size) } > data_end.cast() {
            // not enough room to allocate layout
            return Err(BlockFittingError::NotEnoughAlignedRoom)
        }
        
        // split off into this block, and the new aligned block
        let aligned_block = unsafe { &mut *next_aligned.as_ptr() };
        let aligned_block = aligned_block.write(GCHeapBlockHeader {
            next_free: self.next_free,
            size: usize::from(data_end.addr()) - usize::from(next_aligned.addr()),
            flags: HEADERFLAG_NONE,
            drop_thunk: None
        });
        self.next_free = Some(aligned_block.into());
        self.size = usize::from(next_aligned.addr()) - usize::from(self.data().addr());
        
        //  [self]  |          | [new block] | [layout (aligned)] ... | 
        if unsafe { next_aligned.byte_add(padded_size + size_of::<Self>()).cast() } < data_end {
            // there is enough memory to split off an extra block from the aligned block
            todo!("Split off extra data from aligned block");
            
            return Ok((aligned_block, 2 * size_of::<Self>()))
        }
        
        Ok((aligned_block, size_of::<Self>()))
    }
}
