use std::ptr::{NonNull, Unique};
use std::sync::{mpsc, OnceLock};

use crate::gc::os_dependent::StopAllThreads;

use super::GC_ALLOCATOR;
use super::heap_block_header::GCHeapBlockHeader;

pub(super) static DEALLOCATED_CHANNEL: OnceLock<mpsc::Sender<std::ptr::Unique<[u8]>>> = OnceLock::new();

unsafe fn get_block(data: NonNull<[u8]>) -> NonNull<GCHeapBlockHeader> {
    let data_len = data.len();
    // SAFETY: data needs to be a pointer to a heap allocation
    let block_ptr = unsafe { data.cast::<GCHeapBlockHeader>().byte_sub(size_of::<GCHeapBlockHeader>()) };
    let block_len = unsafe { (*block_ptr.as_ptr()).size };
    assert!(data_len <= block_len, "Length of data (0x{data_len:x}) was larger than the block length (0x{block_len:x})");
    block_ptr
}



pub(super) fn gc_main() -> ! {
    let (sender, reciever) = mpsc::channel::<Unique<[u8]>>();
    DEALLOCATED_CHANNEL.set(sender).expect("Nobody but here sets `DEALLOCATED_CHANNEL`");
    
    // GC CYCLE PROCEDURE:
    //  0. wait until ..? (TODO)
    //  1. Call super::THREAD_LOCAL_ALLOCATORS.write();
    //      - unwrapping is actually fine here, since there *shouldnt* be anywhere to panic during allocations
    //      - TODO: is blocking until we aquire write access okay? I think it might depend on the OS
    //  2. Call `stop_the_world`
    //      - TODO: maybe use a better API, that starts the world on Drop?
    //  3. `GetThreadContext` on all the stopped threads
    //  4. Scan thread registers, stacks, and heap for any root pointers
    //  5. while !roots.is_empty():
    //       let obj = roots.pop_next()
    //       let ptrs = obj.scan_for_ptrs()
    //       roots.extend(ptrs)
    //       scanned.push(obj)
    //  6. for obj in GC_HEAP:
    //       if scanned.contains(obj):
    //         continue
    //       if obj.block().drop_in_place.is_some():
    //         drop_in_place(obj as *mut ())
    //       defer_dealloc(obj)
    //  7. call `start_the_world`
    //  8. work on actually freeing the memory
    
    info!("Starting GC main thread");
    
    loop {
        let to_free = reciever.recv().expect("Sender is stored with 'static lifetime");
        let block = unsafe { get_block(to_free.into()) };
        
        warn!("TODO: free heap block at {block:016x?} (buffer is {:016x?}[0x{:x}])", to_free, to_free.as_ptr().len());
        
        // make sure no threads are currently allocating so we dont deadlock
        let _wl = super::THREAD_LOCAL_ALLOCATORS.write().expect("nowhere should panic during allocations");
        let t = StopAllThreads::new();
        
        for context in t.get_thread_contexts() {
            for ptr in crate::gc::os_dependent::windows::find_filtered(&context, |p| GC_ALLOCATOR.contains(p)) {
                info!("Found pointer to {ptr:016x?} in thread registers")
            }
        }
    }
}
