use std::alloc::Layout;
use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::ptr::NonNull;

use crate::gc::os_dependent::MemorySource;

use super::heap_block_header::GCHeapBlockHeader;
use super::GCAllocatorError;

pub(super) struct TLAllocator<M: MemorySource + 'static> {
    mem_source: &'static M,
    /// The start of this thread's free list.
    /// 
    /// TODO: the GC thread should try to put the freed blocks back into these
    free_list_head: UnsafeCell<Option<NonNull<GCHeapBlockHeader>>>,
}

unsafe impl<M: MemorySource + Sync> Send for TLAllocator<M> {}
impl<M: MemorySource> !Sync for TLAllocator<M> {}

impl<M: MemorySource> TLAllocator<M> {
    pub(super) fn new(source: &'static M) -> Self {
        Self::try_new(source).unwrap()
    }
    
    pub(super) fn try_new(source: &'static M) -> Result<Self, GCAllocatorError> {
        let mem = source.grow_by(1).ok_or(GCAllocatorError::OutOfMemory)?;
        
        // sanity check
        assert!(mem.is_aligned_to(align_of::<GCHeapBlockHeader>()));
        
        let header = mem.cast::<GCHeapBlockHeader>();
        let length = mem.len() - size_of::<GCHeapBlockHeader>();
        
        debug!("Allocated first block at 0x{header:016x?}[0x{length:x}]");
        unsafe { GCHeapBlockHeader::write_new(header.as_ptr(), None, length) };
        
        Ok(Self {
            mem_source: source,
            free_list_head: UnsafeCell::new(Some(header)),
        })
    }
    
    // SAFETY: don't call this method while any mutable references exist to the free list
    unsafe fn find_last_block_mut(&self) -> Option<&mut GCHeapBlockHeader> {
        let mut current =  unsafe { *self.free_list_head.get() }?;
        
        loop {
            // SAFETY: trust me bro (TODO, this is probably justified by the fact that this type is !Sync)
            match unsafe { current.as_mut() }.next_mut() {
                Some(next) => current = next.into(),
                None => break,
            }
        }
        
        // SAFETY: trust me bro (TODO)
        Some(unsafe { &mut *(current.as_ptr()) })
    }
    
    fn has_no_memory(&self) -> bool {
        unsafe { (*self.free_list_head.get()).is_none() }
    }
    
    // Expands the heap by at least the given number of bytes, and returns the block
    fn expand_by(&self, num_bytes: usize) -> Result<NonNull<GCHeapBlockHeader>, GCAllocatorError> {
        let page_size = self.mem_source.page_size();
        let num_pages = num_bytes.div_ceil(page_size);
        let new_ptr = self.mem_source.grow_by(num_pages).ok_or(GCAllocatorError::OutOfMemory)?;
        
        debug!("Expanded heap by 0x{:x} bytes (block @ {:016x?})", new_ptr.len(), new_ptr);
        
        let block_size = new_ptr.len() - size_of::<GCHeapBlockHeader>();
        let block_ptr = new_ptr.cast::<GCHeapBlockHeader>();
        
        unsafe { GCHeapBlockHeader::write_new(block_ptr.as_ptr(), None, block_size) };
        
        // SAFETY: TODO
        match unsafe { self.find_last_block_mut() } {
            None => unsafe {
                self.free_list_head.get().write(Some(block_ptr))
            }
            Some(block) => (*block).next = Some(block_ptr)
        }
        
        Ok(block_ptr)
    }
    
    /// Given a pointer to a heap block in the free list, pop the next one out.
    /// 
    /// If given `None`, pop out the first item from the free list.
    /// 
    /// SAFETY: nobody else can be using the free list!!!
    #[must_use=""]
    unsafe fn pop_next(&self, ptr: Option<NonNull<GCHeapBlockHeader>>) -> Option<NonNull<GCHeapBlockHeader>> {
        match ptr {
            Some(ptr) => unsafe {
                let our_next = &mut (*ptr.as_ptr()).next;
                let old_next = *our_next;
                let new_next = match old_next {
                    None => return None,
                    Some(next) => (*next.as_ptr()).next,
                };
                *our_next = new_next;
                old_next
            }
            None => unsafe {
                let head = &mut *self.free_list_head.get();
                let old_head = *head;
                let new_next = match old_head {
                    None => return None,
                    Some(next) => (*next.as_ptr()).next
                };
                *head = new_next;
                old_head
            }
        }
    }
    
    /// Allocates at least `layout.size()` bytes with alignment of at least `layout.align()`.
    pub(super) fn raw_allocate(&self, layout: Layout) -> Result<(NonNull<GCHeapBlockHeader>, NonNull<[u8]>), GCAllocatorError> {
        if layout.size() == 0 {
            return Err(GCAllocatorError::ZeroSized)
        }
        // TODO: support greater alignment than `16`
        if layout.align() > 16 {
            return Err(GCAllocatorError::AlignmentTooHigh)
        }
        
        // get more memory if needed
        if self.has_no_memory() {
            self.expand_by(layout.size())?;
        }
        assert!(!self.has_no_memory()); // sanity check
        
        // traverse the free list, looking for a block that can handle this layout
        let mut previous: Option<NonNull<_>> = None;
        let mut current = unsafe { *self.free_list_head.get() }.expect("should have memory...");
        loop {
            // SAFETY: nobody else is traversing the free list, since this type is !Sync
            let current_block = unsafe { current.as_mut() };
            
            // sanity check
            assert!(!current_block.is_allocated(), "block @ {:x?} is already allocated", current_block as *const _);
            
            // see if the block can fit `layout` into it
            if current_block.can_allocate(layout) {
                current_block.truncate_and_split(layout.size()).expect(
                    "just checked to make sure this block can allocate"
                );
                break; // we found a block!
            }
            
            // that block didn't work, so lets go to the next one
            previous = Some(current);
            match current_block.next {
                Some(ptr) => current = ptr,
                None => {
                    // we made it all the way to the end of the list and found nothing, just give up
                    // TODO: think of something better to do here
                    return Err(GCAllocatorError::NoBlocksFound)
                },
            }
        }
        
        debug!("Found block @ {:016x?}", current);
        
        // pop out the block from the linked list
        let mut result_block = unsafe { self.pop_next(previous).expect("We know we have a block to pop") };
        // SAFETY: we have exclusive access rn
        let result_block = unsafe { result_block.as_mut() };
        
        // Mark the block as allocated (which also sets `next` to `None`)
        result_block.mark_allocated();
        
        Ok((result_block.into(), result_block.data()))
    }
    
    /// TODO: safety requirements
    unsafe fn raw_allocate_with_drop(&self, layout: Layout, drop_in_place: Option<unsafe fn(*mut ())>) -> Result<NonNull<[u8]>, GCAllocatorError> {
        let (block, data) = self.raw_allocate(layout)?;
        
        // SAFETY: dropper field is in bounds of the allocation, afaik this is similar to using `ptr::offset`
        let drop_ptr = unsafe { &raw mut (*block.as_ptr()).drop_in_place };
        // SAFETY: its fine to write here
        unsafe { drop_ptr.write(drop_in_place) };
        
        Ok(data)
    }
    
    pub(super) fn allocate_for_type<T: Sized>(&self) -> Result<NonNull<MaybeUninit<T>>, GCAllocatorError> {
        // TODO: support allocating dynamically sized types
        #[allow(unsafe_op_in_unsafe_fn)]
        unsafe fn dropper<T>(value: *mut ()) { std::ptr::drop_in_place(value as *mut T) }
        
        let type_layout = std::alloc::Layout::new::<T>();
        
        // using default is fine here. since `<*const T>::Metadata` is `()`, it literally doesnt matter
        let result = unsafe { self.raw_allocate_with_drop(type_layout, Some(dropper::<T>)) }?;
        
        // sanity check
        // SAFETY: length of slice is initialized, and whole slice fits in `isize`
        assert!(unsafe { std::mem::size_of_val_raw(result.as_ptr()) } >= std::mem::size_of::<T>());
        
        // truncate `result_block` to only have the requested size
        let result: NonNull<[u8]> = NonNull::from_raw_parts(result.cast(), type_layout.size());
        
        Ok(result.cast::<MaybeUninit<T>>())
    }
}

