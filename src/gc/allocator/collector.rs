use std::collections::{BinaryHeap, HashSet};
use std::ptr::{NonNull, Unique};
use std::sync::{mpsc, OnceLock};
use std::time::Duration;

use thread_local::ThreadLocal;
use windows_sys::Win32::System::Threading::GetThreadId;

use crate::gc::os_dependent::windows::heap_scan::WinHeapLock;
use crate::gc::os_dependent::windows::{get_all_threads, get_thread_stack_bounds, get_writable_segments};
use crate::gc::os_dependent::{MemorySource, StopAllThreads};

use super::tl_allocator::TLAllocator;
use super::{GC_ALLOCATOR, MEMORY_SOURCE, MemorySourceImpl};
use super::heap_block_header::GCHeapBlockHeader;

pub(super) static DEALLOCATED_CHANNEL: OnceLock<mpsc::Sender<std::ptr::Unique<[u8]>>> = OnceLock::new();

fn get_block_from_data(data: NonNull<[u8]>) -> NonNull<GCHeapBlockHeader> {
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

unsafe fn scan_segment(data: NonNull<[u8]>) -> impl IntoIterator<Item=*const ()> {
    gen move {
        let (base, len) = data.to_raw_parts();
        let base = base.cast::<*const ()>();
        let len = len * size_of::<u8>() / size_of::<*const ()>();
        for i in 0..len {
            let value = unsafe { base.add(i).read_volatile() };
            if MEMORY_SOURCE.contains(value) {
                yield value
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
    let (block_ptr, heap_size) = MEMORY_SOURCE.raw_heap_data().to_raw_parts();
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
        
        while root >= next_block.as_ptr().cast() {
            block_ptr = next_block;
            current_block = unsafe { block_ptr.as_mut() };
            trace!("Traversing block {block_ptr:016x?}[0x{:x}]", current_block.size);
            next_block = current_block.next();
        }
        if block_ptr >= end { break }
        
        assert!(root >= block_ptr.as_ptr().cast());
        let block_range_len = size_of::<GCHeapBlockHeader>() + current_block.size;
        
        // NOTE: if there is a pointer DIRECTLY to a given block header,
        // then it almost certainly is an internal GC thing thats just stored on the heap  
        if root == block_ptr.as_ptr().cast() {
            info!("found direct free block pointer ({root:016x?}[{block_range_len:x}])");
            continue
        }
        
        if !current_block.is_allocated() {
            warn!("dangling pointer detected ({root:016x?} points to block {block_ptr:016x?}[{block_range_len:x}], which is free)");
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
    if block_ptr != end {
        error!("Heap corruption detected (expected to end at {end:016x?}, got {block_ptr:016x?})")
    }
    
    marked_blocks
}

fn scan_block(block: &GCHeapBlockHeader) -> impl IntoIterator<Item=*const ()> {
    gen {
        let (ptr, len) = block.data().to_raw_parts();
        let ptr = ptr.cast::<*const ()>();
        
        let n = len / size_of::<*const ()>();
        for i in 0..n {
            let value = unsafe { ptr.add(i).read() };
            if MEMORY_SOURCE.contains(value) {
                yield value;
            }
        }
    }
}

fn get_block(ptr: *const ()) -> Option<NonNull<GCHeapBlockHeader>> {
    if !MEMORY_SOURCE.contains(ptr) {
        return None
    }
    
    let (block_ptr, heap_size) = MEMORY_SOURCE.raw_heap_data().to_raw_parts();
    let end = unsafe { block_ptr.byte_add(heap_size) };
    let mut block_ptr = block_ptr.cast::<GCHeapBlockHeader>();
    
    while block_ptr < end.cast() {
        if ptr > block_ptr.as_ptr().cast() { return Some(block_ptr) }
        block_ptr = unsafe { block_ptr.as_ref() }.next();
    }
    
    None
}

fn get_live_blocks(roots: Vec<NonNull<GCHeapBlockHeader>>) -> HashSet<NonNull<GCHeapBlockHeader>> {
    use std::collections::BTreeSet;
    let mut scanned = HashSet::<NonNull<GCHeapBlockHeader>>::with_capacity(roots.capacity());
    let mut roots = BTreeSet::from_iter(roots); // should be fast bc roots is sorted
    
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

fn destruct_block_data(block: &mut GCHeapBlockHeader) -> Result<(), Box<dyn std::any::Any + Send>> {
    let drop_in_place = block.drop_in_place;
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
    let mut blocks = blocks.into_iter();
    
    // TODO: allocate blocks to each thread actually intelligently
    while let Some(block) = blocks.next() {
        let min_thread = prio_queue.pop().expect("Should be more than zero threads");
        min_thread.0.reclaim_block(block);
        prio_queue.push(min_thread);
    }
}

fn sweep_heap(live_blocks: HashSet<NonNull<GCHeapBlockHeader>>) -> impl IntoIterator<Item=NonNull<GCHeapBlockHeader>> {
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
        let heap = crate::gc::os_dependent::windows::heap_scan::WinHeap::new().unwrap();
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
        
        debug!("Rooted blocks: {root_blocks:016x?}");
        
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
            reciever.try_iter().map(|p| get_block_from_data(p.into())),
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
