use super::{MEMORY_SOURCE, super::MemorySource};
use super::GCHeapBlockHeader;
use std::collections::HashSet;
use std::ptr::NonNull;

fn destruct_block_data(block: &mut GCHeapBlockHeader) -> Result<(), Box<dyn std::any::Any + Send>> {
    let drop_in_place = block.drop_thunk;
    let data_ptr = block.data().cast::<()>();
    
    let drop_in_place = match drop_in_place { None => return Ok(()), Some(d) => d };
    
    match std::panic::catch_unwind(|| {
        // TODO: prevent all the other evil stuff from happening here
        // Including but not limited to:
        //  - storing currently destructing pointers in statics, heap, stack, or wherever else
        //  - spawning more threads
        unsafe { drop_in_place(data_ptr.as_ptr()) }
    }) {
        Ok(()) => Ok(()),
        Err(payload) => {
            // See [`std::panicking::payload_as_str`]
            let s = if let Some(&s) = payload.downcast_ref::<&'static str>() {
                s
            } else if let Some(s) = payload.downcast_ref::<String>() {
                s.as_str()
            } else {
                "Box<dyn Any>"
            };
            error!("Panic in destructor: {s}");
            Err(payload)
        }
    }
}

pub(super) fn sweep_heap(live_blocks: HashSet<NonNull<GCHeapBlockHeader>>) -> impl IntoIterator<Item=NonNull<GCHeapBlockHeader>> {
    gen move {
        let (block_ptr, heap_size) = MEMORY_SOURCE.raw_heap_data().to_raw_parts();
        let end = unsafe { block_ptr.byte_add(heap_size) };
        let mut block_ptr = block_ptr.cast::<GCHeapBlockHeader>();
        
        while block_ptr < end.cast() {
            let next_block = unsafe { block_ptr.as_ref() }.next();
            
            if !unsafe { block_ptr.as_ref().is_allocated() } {
                // not even allocated, dont free it again lol
                block_ptr = next_block;
                continue
            }
            
            if live_blocks.contains(&block_ptr) {
                block_ptr = next_block;
                continue // can't free this yet
            }
            
            trace!("Freeing block {block_ptr:016x?}");
            
            // run destructor (evil)
            let _panic_payload = destruct_block_data(unsafe { block_ptr.as_mut() });
            
            // TODO: check to make sure the destructor didn't do anything evil.
            //       if it did, just `std::process::exit(1)` or something.
            
            // Actually mark the stuff as freed
            yield block_ptr;
            
            // go to the next
            block_ptr = next_block;
        }
        
        if block_ptr != end.cast() {
            error!("Heap corruption detected (expected to end at {end:016x?}, got {block_ptr:016x?})")
        }
    }
}
