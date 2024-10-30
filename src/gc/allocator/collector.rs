use std::ptr::{NonNull, Unique};
use std::sync::{mpsc, OnceLock};
use std::time::Duration;

use windows_sys::Win32::System::Threading::GetThreadId;

use crate::gc::os_dependent::windows::heap_scan::WinHeapLock;
use crate::gc::os_dependent::windows::{get_all_threads, get_thread_stack_bounds};
use crate::gc::os_dependent::{MemorySource, StopAllThreads};

use super::{GC_ALLOCATOR, MEMORY_SOURCE, MemorySourceImpl};
use super::heap_block_header::GCHeapBlockHeader;

pub(super) static DEALLOCATED_CHANNEL: OnceLock<mpsc::Sender<std::ptr::Unique<[u8]>>> = OnceLock::new();


fn get_block(data: NonNull<[u8]>) -> NonNull<GCHeapBlockHeader> {
    let data_len = data.len();
    // SAFETY: data needs to be a pointer to a heap allocation
    let block_ptr = unsafe { data.cast::<GCHeapBlockHeader>().byte_sub(size_of::<GCHeapBlockHeader>()) };
    let block_len = unsafe { (*block_ptr.as_ptr()).size };
    assert!(data_len <= block_len, "Length of data (0x{data_len:x}) was larger than the block length (0x{block_len:x})");
    block_ptr
}


pub fn scan_registers(c: &windows_sys::Win32::System::Diagnostics::Debug::CONTEXT) -> impl IntoIterator<Item=*const ()> {
    gen move {
        let n = size_of_val(c) / size_of::<*const ()>();
        let ptr = c as *const _ as *const *const ();
        for i in 0..n {
            let x = unsafe { ptr.add(i).read() };
            if MEMORY_SOURCE.contains(x) {
                yield x
            }
        }
    }
}

unsafe fn scan_stack(bounds: (*const (), *const ()), rsp: *const ()) -> impl IntoIterator<Item=*const ()> {
    gen move {
        let (top, base) = bounds;
        assert!(top < base, "stack always grows downwards");
        assert!(top < rsp && rsp < base, "rsp should be between top and base");
        let (_top, base, rsp) = (top as *const *const (), base as *const *const (), rsp as *const *const ());
        let n = unsafe { base.offset_from(rsp) } as usize;
        for i in 0..n {
            let x = unsafe { rsp.add(i).read_volatile() };
            if MEMORY_SOURCE.contains(x) {
                yield x
            }
        }
    }
}

fn scan_heap(roots: &mut Vec<*const ()>, mut lock: WinHeapLock) {
    // TODO: tune these values
    const MINIMUM_CAP: usize = 64;
    const GROWTH_FACTOR: usize = 4;
    
    let initial_length = roots.len();
    'main: loop {
        // Allocate more if the vector is full
        if roots.len() == roots.capacity() {
            lock.with_unlocked(|| {
                let num_to_reserve = std::cmp::max(MINIMUM_CAP - roots.len(), (GROWTH_FACTOR - 1) * roots.capacity()); 
                roots.reserve(num_to_reserve)
            })
        }
        
        for b in lock.walk() {
            if !b.is_allocated() { continue }
            let block_data = b.data().cast::<*const ()>();
            
            if block_data == roots.as_ptr().cast() {
                // we found the allocation containing our roots vector LOL
                continue
            }
            
            let n = b.data_size() / size_of::<*const ()>();
            for i in 0..n {
                let ptr = unsafe { block_data.add(i).read_volatile() };
                if MEMORY_SOURCE.contains(ptr) {
                    debug!("Found pointer to {ptr:016x?} in heap (at address {:016x?})", block_data.wrapping_add(i));
                    match roots.push_within_capacity(ptr) {
                        Ok(()) => (),
                        Err(_) => {
                            // we need to rescan the whole heap, since we are gonna allocate more
                            roots.truncate(initial_length);
                            continue 'main
                        }
                    }
                }
            }
        }
        
        break
    }
}

fn get_root_blocks(roots: Vec<*const ()>) -> Vec<NonNull<GCHeapBlockHeader>> {
    let (mut block_ptr, heap_size) = MEMORY_SOURCE.raw_heap_data().to_raw_parts();
    let end = unsafe { block_ptr.byte_add(heap_size) };
    
    debug_assert!(roots.is_sorted());
    let mut root_idx = 0;
    
    let mut marked = Vec::new();
    
    while block_ptr < end { // NOTE: can add `&& root_idx < roots.len()` to avoid full validation
        let current_block = unsafe { block_ptr.cast::<GCHeapBlockHeader>().as_mut() };
        
        if current_block.size == 0 {
            error!("Heap corruption detected at block {block_ptr:016x?}: allocations of size zero should not exist")
        }
        
        trace!("Traversing block {block_ptr:016x?}[0x{:x}]", current_block.size);
        
        let next_block = current_block.next().cast::<()>();
        
        while let Some(&root) = roots.get(root_idx) && root < next_block.as_ptr() {
            assert!(block_ptr.as_ptr().cast_const() <= root);
            
            if !current_block.is_allocated() {
                let block_range_len = size_of::<GCHeapBlockHeader>() + current_block.size;
                // NOTE: if there is a pointer DIRECTLY to a given block header,
                // then it almost certainly is an internal GC thing thats just stored on the heap
                if root == block_ptr.as_ptr() {
                    info!("found direct free block pointer ({root:016x?}[{block_range_len:x}])")
                } else {
                    warn!("dangling pointer detected ({root:016x?} points to block {block_ptr:016x?}[{block_range_len:x}], which is free)");
                }
            }
            
            if current_block.is_allocated() && marked.last() != Some(&block_ptr.cast()) {
                debug!("Marked block @ {block_ptr:016x?} (pointer was {root:016x?})");
                marked.push(block_ptr.cast());
            }
            
            root_idx += 1;
        }
        
        block_ptr = next_block;
    }
    debug!("Done marking roots");
    if block_ptr != end {
        error!("Heap corruption detected (expected to end at {end:016x?}, got {block_ptr:016x?})")
    }
    
    marked
}

fn get_dead_blocks(roots: Vec<NonNull<GCHeapBlockHeader>>) -> Vec<NonNull<GCHeapBlockHeader>> {
    warn!("TODO: get all the dead blocks given the roots");
    vec![]
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
        let heap = crate::gc::os_dependent::windows::heap_scan::WinHeap::new().unwrap();
        let heap_lock = heap.lock().unwrap();
        let mut _wl = super::THREAD_LOCAL_ALLOCATORS.write().expect("nowhere should panic during allocations");
        let t = StopAllThreads::new();
        
        std::thread::sleep(Duration::from_millis(20));
        
        // Scan for roots ------------------------------
        let mut roots = Vec::new();
        
        // Scan heap
        scan_heap(&mut roots, heap_lock);
        // NOTE: we can allocate without deadlocking again since `heap_lock` got used
        
        // Scan global (mutable) static memory
        warn!("TODO: Scan static variables");
        
        // Scan each thread's memory
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
        
        let root_blocks = get_root_blocks(roots);
        
        info!("finished getting rooted blocks");
        
        // Scan the GC heap, starting from the roots
        let _dead_blocks = get_dead_blocks(root_blocks);
        
        // NOTE: if it werent for absolutely stupid Drop implementations,
        // we could soundly let all the threads go *now*, and asynchronously
        // start dropping and freeing up all the dead stuff. but since people
        // can (and DO) put literally everything in Drop, we have to run them
        // in a controlled environment where we can make sure they arent
        // creating dangling references. (NOTE: you can also start new threads
        // during Drop. i know this is a problem, but idk how much yet. at the
        // LEAST we have to monitor all memory accesses during it, but idk how)
        
        // Free everything that wasn't found during the heap traversal
        let _freeable_ptrs = reciever.try_iter().map(|p| get_block(p.into()));
        warn!("TODO: drop everything in dead_blocks");
        warn!("TODO: free everything in dead_blocks and freeable_ptrs");
        
        // Wake any threads waiting for garbage to have been cleaned up
        *super::GC_CYCLE_NUMBER.try_lock().unwrap() += 1;
        super::GC_CYCLE_SIGNAL.notify_all();
        
        info!("Finished garbage collection");
    }
}
