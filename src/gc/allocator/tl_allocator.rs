use std::alloc::Layout;
use std::cell::Cell;
use std::mem::MaybeUninit;
use std::ptr::NonNull;

use crate::gc::allocator::heap_block_header::HEADERFLAG_NONE;

use super::os_dependent::MemorySource;

use super::heap_block_header::GCHeapBlockHeader;
use super::GCAllocatorError;

pub(super) struct TLAllocator<M: MemorySource + 'static> {
    memory_source: &'static M,
    /// The start of this thread's free list.
    /// 
    /// TODO: the GC thread should try to put the freed blocks back into these
    free_list_head: Cell<Option<NonNull<GCHeapBlockHeader>>>,
    /// The amount of free memory this allocator has.
    num_free_bytes: Cell<usize>,
    /// A list of blocks that this allocator got
    alloced_blocks: Cell<Option<Vec<NonNull<[u8]>>>>,
}

unsafe impl<M: MemorySource + Sync> Send for TLAllocator<M> {}
impl<M: MemorySource> !Sync for TLAllocator<M> {}

// Methods used externally
impl<M: MemorySource> TLAllocator<M> {
    pub(super) fn allocate_for_value<T: Sized>(&self, value: T) -> Result<NonNull<T>, (GCAllocatorError, T)> {
        // TODO: support allocating dynamically sized types
        
        if size_of::<T>() == 0 {
            return Ok(NonNull::dangling())
        }
        
        #[allow(unsafe_op_in_unsafe_fn)]
        unsafe fn dropper<T>(value: *mut ()) { std::ptr::drop_in_place(value as *mut T) }
        
        let type_layout = std::alloc::Layout::new::<T>();
        
        let result = unsafe { self.raw_allocate_with_drop(type_layout, Some(dropper::<T>)) };
        
        let result = match result {
            Ok(r) => r,
            Err(e) => return Err((e, value))
        };
        
        // sanity check
        // SAFETY: length of slice is initialized, and whole slice fits in `isize`
        assert!(unsafe { std::mem::size_of_val_raw(result.as_ptr()) } >= std::mem::size_of::<T>());
        
        let result = result.cast::<T>();
        
        // SAFETY: result can hold a `T`
        unsafe { result.write(value) };
        
        Ok(result)
    }
}

impl<M: MemorySource> TLAllocator<M> {
    pub(super) fn try_new(source: &'static M) -> Result<Self, GCAllocatorError> {
        let mem = source.grow_by(1).ok_or(GCAllocatorError::OutOfMemory)?;
        
        // sanity check
        assert!(mem.is_aligned_to(align_of::<GCHeapBlockHeader>()));
        
        let header = unsafe { mem.cast::<MaybeUninit<GCHeapBlockHeader>>().as_mut() };
        let length = mem.len() - size_of::<GCHeapBlockHeader>();
        
        debug!("Allocated first block at 0x{:016x?}[0x{length:x}]", header.as_ptr());
        let header = header.write(GCHeapBlockHeader {
            next_free: None,
            size: length,
            flags: HEADERFLAG_NONE,
            drop_thunk: None
        });
        
        Ok(Self {
            memory_source: source,
            free_list_head: Cell::new(Some(header.into())),
            num_free_bytes: Cell::new(length),
            alloced_blocks: Cell::new(Some(vec![mem])),
        })
    }
    
    /// The total number of free bytes in the heap
    pub(super) fn free_bytes(&self) -> usize {
        self.num_free_bytes.get()
    }
    
    /// Whether the heap has ZERO free memory
    fn has_no_memory(&self) -> bool {
        assert_eq!(self.free_list_head.get().is_none(), self.free_bytes() == 0);
        self.free_list_head.get().is_none()
    }
    
    // Expands the heap by at least the given number of bytes, and returns the block
    fn expand_by(&self, num_bytes: usize, last_block: Option<&mut GCHeapBlockHeader>) -> Result<NonNull<GCHeapBlockHeader>, GCAllocatorError> {
        // Get (at least) the requested amount of memory
        let page_size = self.memory_source.page_size();
        let num_pages = (num_bytes + size_of::<GCHeapBlockHeader>()).div_ceil(page_size);
        let new_ptr = self.memory_source.grow_by(num_pages).ok_or(GCAllocatorError::OutOfMemory)?;
        
        debug!("Expanded heap by 0x{:x} bytes (block @ {:016x?})", new_ptr.len(), new_ptr);
        
        // Add this block to the allocated block list
        let mut blocks = self.alloced_blocks.replace(None).expect("");
        blocks.push(new_ptr);
        self.alloced_blocks.set(Some(blocks));
        
        // initialize the block header
        let block_size = new_ptr.len() - size_of::<GCHeapBlockHeader>();
        let block_ptr = new_ptr.cast::<GCHeapBlockHeader>();
        
        unsafe { 
            block_ptr.write(GCHeapBlockHeader {
                next_free: None,
                size: block_size,
                flags: HEADERFLAG_NONE,
                drop_thunk: None
            });
        }
        
        match last_block {
            None => self.free_list_head.set(Some(block_ptr)),
            Some(block) => block.next_free = Some(block_ptr)
        }
        
        // Update the amount of free bytes we have
        self.num_free_bytes.update(|n| n + block_size);
        
        Ok(block_ptr)
    }
    
    /// Adds a block into the heap.
    pub(super) fn reclaim_block(&mut self, mut block_ptr: NonNull<GCHeapBlockHeader>) {
        let block = unsafe { block_ptr.as_mut() };
        self.num_free_bytes.update(|n| n + block.size);
        self.free_list_head.update(|old| {
            block.set_free(old);
            Some(block_ptr)
        });
    }
    
    /// Given a pointer to a heap block in the free list, pop the next one out.
    /// 
    /// If given `None`, pop out the first item from the free list.
    /// 
    /// SAFETY: nowhere else can be using the free list!!!
    unsafe fn pop_next(&self, ptr: Option<NonNull<GCHeapBlockHeader>>) -> Option<NonNull<GCHeapBlockHeader>> {
        match ptr {
            Some(ptr) => unsafe {
                let our_next = &mut (*ptr.as_ptr()).next_free;
                let old_next = *our_next;
                let new_next = match old_next {
                    None => return None,
                    Some(next) => (*next.as_ptr()).next_free,
                };
                *our_next = new_next;
                old_next
            }
            None => {
                let old_head = self.free_list_head.get();
                let new_next = match old_head {
                    None => return None,
                    Some(next) => unsafe { next.as_ref().next_free }
                };
                self.free_list_head.set(new_next);
                old_head
            }
        }
    }
    
    /// Finds (or creates) a block to fit `layout`, and pops it out of the free list.
    fn find_good_block(&self, layout: Layout) -> Result<&mut GCHeapBlockHeader, GCAllocatorError> {
        // traverse the free list, looking for a block that can handle this layout
        let mut previous: Option<NonNull<_>> = None;
        let mut current = self.free_list_head.get().expect("should have some free memory...");
        
        loop {
            // SAFETY: nobody else is traversing the free list, since this type is !Sync
            let current_block = unsafe { current.as_mut() };
            
            // sanity check
            assert!(!current_block.is_allocated(), "block @ {:x?} is already allocated", current_block as *const _);
            
            // see if the block can fit `layout` into it
            if let Ok((block, new_header_bytes)) = current_block.shrink_to_fit(layout) {
                // check if we split off a block from the beginning, if so, update `previous`
                if current != block.into() {
                    assert_eq!(unsafe { (*current.as_ptr()).next_free }, Some(block.into())); // sanity check
                    previous = Some(current);
                    current = block.into();
                }
                
                // we split off a block from the end, so update that
                self.num_free_bytes.update(|n| n.checked_sub(new_header_bytes).expect("should have enough bytes"));
                
                // either way, we found a block!
                break
            }
            
            // that block didn't work, so lets go to the next one
            previous = Some(current);
            match current_block.next_free {
                Some(ptr) => current = ptr,
                None => {
                    // we made it all the way to the end of the list and found nothing, so add more memory
                    current = self.expand_by(layout.size(), Some(current_block))?;
                },
            }
        }
        
        trace!("Found block @ {:016x?}", current);
        
        // pop out the block from the linked list
        let mut result_block = unsafe { self.pop_next(previous).expect("We know we have a block to pop") };
        // SAFETY: we have exclusive access rn
        let result_block = unsafe { result_block.as_mut() };
        
        // Mark the block as allocated (which also sets `next` to `None`)
        result_block.set_allocated();
        self.num_free_bytes.update(|n| n.checked_sub(result_block.size).expect("should have free bytes in block"));
        
        Ok(result_block)
    }
    
    /// Allocates at least `layout.size()` bytes with alignment of at least `layout.align()`.
    pub(super) fn raw_allocate(&self, layout: Layout) -> Result<(&mut GCHeapBlockHeader, NonNull<[u8]>), GCAllocatorError> {
        if layout.size() == 0 {
            return Err(GCAllocatorError::ZeroSized)
        }
        // TODO: support greater alignment than `16`
        if layout.align() > 16 {
            return Err(GCAllocatorError::BadAlignment)
        }
        
        // get more memory if needed
        if self.free_bytes() < layout.size() {
            self.expand_by(layout.size(), None)?;
        }
        
        assert!(!self.has_no_memory()); // sanity check
        
        let result_block = self.find_good_block(layout)?;
        let data = result_block.data();
        
        Ok((result_block, data))
    }
    
    /// TODO: safety requirements
    unsafe fn raw_allocate_with_drop(&self, layout: Layout, drop_in_place: Option<unsafe fn(*mut ())>) -> Result<NonNull<[u8]>, GCAllocatorError> {
        let (block, data) = self.raw_allocate(layout)?;
        
        block.drop_thunk = drop_in_place;
        
        Ok(data)
    }
}

