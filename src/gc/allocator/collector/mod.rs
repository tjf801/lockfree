use std::collections::{BinaryHeap, HashSet};
use std::ptr::{NonNull, Unique};
use std::sync::{mpsc, OnceLock};
use std::time::Duration;

use thread_local::ThreadLocal;
use windows_sys::Win32::System::Threading::GetThreadId;

use super::os_dependent::windows::{get_all_threads, get_thread_stack_bounds, get_writable_segments};
use super::os_dependent::{MemorySource, StopAllThreads};

use super::tl_allocator::TLAllocator;
use super::{get_block, MEMORY_SOURCE, MemorySourceImpl};
use super::heap_block_header::GCHeapBlockHeader;

mod scanning;
mod sweeping;

use scanning::{scan_block, scan_heap, scan_registers, scan_segment, scan_stack};
use sweeping::sweep_heap;

// NOTE: this has to be `Unique` since `NonNull` is not `Send`. why does rust
// do this with raw pointers come onnnn its not even needed
pub(super) static DEALLOCATED_CHANNEL: OnceLock<mpsc::Sender<std::ptr::Unique<[u8]>>> = OnceLock::new();

fn get_root_blocks(roots: Vec<*const ()>) -> impl IntoIterator<Item=NonNull<GCHeapBlockHeader>> {
    let (block_ptr, heap_size) = MEMORY_SOURCE.raw_data().to_raw_parts();
    let mut block_ptr = block_ptr.cast::<GCHeapBlockHeader>();
    trace!("Traversing block {block_ptr:016x?}[0x{:x}]", unsafe { block_ptr.as_ref().size });
    let end = unsafe { block_ptr.byte_add(heap_size) };
    
    debug_assert!(roots.is_sorted());
    
    let mut marked_blocks = Vec::new();
    
    for root in roots.into_iter() {
        let mut current_block = unsafe { block_ptr.as_mut() };
        let mut next_block = current_block.next();
        
        if current_block.size == 0 {
            error!("Heap corruption detected at block {block_ptr:016x?}: allocations of size zero should not exist")
        }
        
        while root.cast() >= next_block.as_ptr() {
            block_ptr = next_block;
            current_block = unsafe { block_ptr.as_mut() };
            trace!("Traversing block {block_ptr:016x?}[0x{:x}]", current_block.size);
            next_block = current_block.next();
        }
        if block_ptr >= end { break }
        
        assert!(root.cast() >= block_ptr.as_ptr());
        let block_range_len = size_of::<GCHeapBlockHeader>() + current_block.size;
        
        // NOTE: if there is a pointer DIRECTLY to a given block header,
        // then it almost certainly is an internal GC thing thats just stored on the heap  
        if root.cast() == block_ptr.as_ptr() {
            info!("found direct free block pointer ({root:016x?}[{block_range_len:x}])");
            continue
        }
        
        if !current_block.is_allocated() {
            warn!("dangling pointer detected ({root:016x?} points to block {block_ptr:016x?}[{block_range_len:x}], which is free)");
            // std::process::exit(1);
            continue
        }
        
        if marked_blocks.last() == Some(&block_ptr.cast()) {
            // we just got a pointer to it
            trace!("Ignoring additional pointer to {block_ptr:016x?} (just marked it)");
            continue
        }
        
        debug!("Marked block @ {block_ptr:016x?} (pointer was {root:016x?})");
        marked_blocks.push(block_ptr);
    }
    debug!("Done marking roots");
    
    marked_blocks
}


/// Returns all the live blocks on the GC heap.
fn get_live_blocks(roots: impl IntoIterator<Item=NonNull<GCHeapBlockHeader>>) -> HashSet<NonNull<GCHeapBlockHeader>> {
    use std::collections::BTreeSet;
    let mut roots = BTreeSet::from_iter(roots); // should be fast bc roots is sorted
    let mut scanned = HashSet::<NonNull<GCHeapBlockHeader>>::with_capacity(roots.len()*2);
    
    debug!("Rooted blocks: {roots:016x?}");
    
    while let Some(block) = roots.pop_first() {
        let block_ref = unsafe { block.as_ref() };
        
        for new_ptr in scan_block(block_ref).into_iter() {
            debug!("Found new live pointer in GC heap {new_ptr:016x?}");
            let block: NonNull<GCHeapBlockHeader> = get_block(new_ptr).expect("scan_block only gives pointers that we know are in the GC heap");
            if !scanned.contains(&block) {
                roots.insert(block);
            }
        }
        
        scanned.insert(block);
    }
    
    scanned
}

fn free_blocks(
    blocks: impl IntoIterator<Item=NonNull<GCHeapBlockHeader>>,
    tl_allocs: &mut ThreadLocal<TLAllocator<MemorySourceImpl>>
) {
    struct FreeByteComparer<'a>(&'a mut TLAllocator<MemorySourceImpl>);
    impl PartialEq for FreeByteComparer<'_> {
        fn eq(&self, other: &Self) -> bool { self.0.free_bytes().eq(&other.0.free_bytes()) }
    }
    impl Eq for FreeByteComparer<'_> {}
    impl PartialOrd for FreeByteComparer<'_> {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> { Some(self.cmp(other)) }
    }
    impl Ord for FreeByteComparer<'_> {
        fn cmp(&self, other: &Self) -> std::cmp::Ordering { other.0.free_bytes().cmp(&self.0.free_bytes()) }
    }
    
    let mut prio_queue: BinaryHeap<FreeByteComparer> = BinaryHeap::from_iter(tl_allocs.iter_mut().map(FreeByteComparer));
    let blocks = blocks.into_iter();
    
    // TODO: allocate blocks to each thread actually intelligently
    for block in blocks {
        let min_thread = prio_queue.pop().expect("Should be more than zero threads");
        min_thread.0.reclaim_block(block);
        prio_queue.push(min_thread);
    }
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
    
    'main: loop {
        // TODO: make a better way to know when to GC
        std::thread::sleep(Duration::from_secs(2));
        
        // make sure no threads are currently allocating so we dont deadlock
        info!("Starting GC Cycle");
        let heap = super::os_dependent::windows::heap_scan::WinHeap::new().unwrap();
        let heap_lock = heap.lock().unwrap();
        let mut tl_allocators = super::THREAD_LOCAL_ALLOCATORS.write().expect("nowhere should panic during allocations");
        let t = StopAllThreads::new();
        
        std::thread::sleep(Duration::from_millis(20));
        
        // Scan for roots ------------------------------
        let mut roots = Vec::new();
        
        // Scan heap
        info!("Scanning process heap");
        scan_heap(&mut roots, heap_lock);
        // NOTE: we can allocate without deadlocking again since `heap_lock` got used
        
        // Scan global (mutable) static memory
        for (name, segment_data) in get_writable_segments() {
            info!("Scanning {name} segment");
            for root in unsafe { scan_segment(segment_data) } {
                debug!("Found pointer to {root:016x?} in {name} segment");
                roots.push(root);
            }
        }
        
        // Scan each thread's memory
        info!("Scanning threads");
        for thread in get_all_threads().into_iter().map(Result::unwrap) {
            let id = unsafe { GetThreadId(thread) };
            debug!("Scanning thread {id:x?}");
            
            // Scan thread registers
            let context = match unsafe { t.get_thread_context(thread) } {
                Ok(c) => c,
                Err(code) => {
                    error!("Collector: get_thread_context failed with code {code:x}");
                    continue 'main
                }
            };
            for ptr in scan_registers(&context) {
                debug!("Found pointer to {ptr:016x?} in thread registers");
                roots.push(ptr);
            }
            
            // scan thread stacks
            let bounds = get_thread_stack_bounds(thread).unwrap();
            let stack_ptr = bounds.0.with_addr(context.Rsp as usize) as *const ();
            for ptr in unsafe { scan_stack(bounds, stack_ptr) } {
                debug!("Found pointer to {ptr:016x?} in thread stack");
                roots.push(ptr);
            }
            
            // TODO: scan thread local storage
        }
        warn!("TODO: Scan thread local storage");
        
        roots.sort();
        roots.dedup();
        
        debug!("Root pointers: {roots:016x?}");
        
        let root_blocks = get_root_blocks(roots);
        
        info!("finished getting rooted blocks");
        
        // Scan the GC heap, starting from the roots
        let live_blocks = get_live_blocks(root_blocks);
        
        debug!("Live blocks ({}): {live_blocks:016x?}", live_blocks.len());
        
        // NOTE: if it werent for absolutely stupid Drop implementations,
        // we could soundly let all the threads go *now*, and asynchronously
        // start dropping and freeing up all the dead stuff. but since people
        // can (and DO) put literally everything in Drop, we have to run them
        // in a controlled environment where we can make sure they arent
        // creating dangling references. (NOTE: you can also start new threads
        // during Drop. i know this is a problem, but idk how much yet. at the
        // LEAST we have to monitor all memory accesses during it, but idk how)
        
        // Free everything that we know we can free (bc we recieved them over the channel)
        free_blocks(
            reciever.try_iter().map(|data| {
                let data = NonNull::from(data);
                let data_len = data.len();
                // SAFETY: data needs to be a pointer to a heap allocation
                let block_ptr = unsafe { data.cast::<GCHeapBlockHeader>().byte_sub(size_of::<GCHeapBlockHeader>()) };
                let block_len = unsafe { (*block_ptr.as_ptr()).size };
                assert!(data_len <= block_len, "Length of data (0x{data_len:x}) was larger than the block length (0x{block_len:x})");
                block_ptr
            }),
            &mut tl_allocators
        );
        
        info!("Freed explicit deallocations");
        
        // sweep (i.e: drop) and free the rest of the dead stuff in the heap
        free_blocks(sweep_heap(live_blocks), &mut tl_allocators);
        
        info!("Freed all dead blocks");
        
        // Wake any threads waiting for garbage to have been cleaned up
        *super::GC_CYCLE_NUMBER.try_lock().unwrap() += 1;
        super::GC_CYCLE_SIGNAL.notify_all();
        
        info!("Finished garbage collection");
    }
}
