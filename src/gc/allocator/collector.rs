use std::ptr::{NonNull, Unique};
use std::sync::mpsc::TryRecvError;
use std::sync::{mpsc, OnceLock};
use std::time::Duration;

use windows_sys::Win32::System::Threading::GetThreadId;

use crate::gc::os_dependent::windows::{get_all_threads, get_thread_stack_bounds};
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


pub fn scan_registers<'a, F>(c: &'a windows_sys::Win32::System::Diagnostics::Debug::CONTEXT, mut func: F) -> impl IntoIterator<Item=*const ()> where F: FnMut(*const ()) -> bool + 'a {
    gen move {
        let n = size_of_val(c) / size_of::<*const ()>();
        let ptr = c as *const windows_sys::Win32::System::Diagnostics::Debug::CONTEXT as *const *const ();
        for i in 0..n {
            let x = unsafe { ptr.add(i).read() };
            if func(x) {
                yield x
            }
        }
    }
}

unsafe fn scan_stack<F: FnMut(*const ()) -> bool>(mut func: F, bounds: (*const (), *const ()), rsp: *const ()) -> impl IntoIterator<Item=*const ()> {
    gen move {
        let (top, base) = bounds;
        assert!(top < base, "stack always grows downwards");
        assert!(top < rsp && rsp < base, "rsp should be between top and base");
        let (_top, base, rsp) = (top as *const *const (), base as *const *const (), rsp as *const *const ());
        let n = unsafe { base.offset_from(rsp) } as usize;
        for i in 0..n {
            let x = unsafe { rsp.add(i).read() };
            if func(x) {
                yield x
            }
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
    
    loop {
        std::thread::sleep(Duration::from_secs(2));
        
        let mut ptrs = vec![];
        while let Ok(to_free) = reciever.try_recv() {
            let block = unsafe { get_block(to_free.into()) };
            warn!("TODO: free heap block at {block:016x?} (buffer is {:016x?}[0x{:x}])", to_free, to_free.as_ptr().len());
            ptrs.push(to_free);
        }
        
        // make sure no threads are currently allocating so we dont deadlock
        let _wl = super::THREAD_LOCAL_ALLOCATORS.write().expect("nowhere should panic during allocations");
        let t = StopAllThreads::new();
        
        std::thread::sleep(Duration::from_millis(20));
        
        for thread in get_all_threads().into_iter().map(Result::unwrap) {
            let id = unsafe { GetThreadId(thread) };
            debug!("scanning thread {id:x?}");
            
            let contains = |p| GC_ALLOCATOR.contains(p);
            
            // Scan thread registers
            let context = unsafe { t.get_thread_context(thread).unwrap() };
            for ptr in scan_registers(&context, contains) {
                let block = ptr.wrapping_byte_offset(-20);
                info!("Found pointer to {ptr:016x?} (maybe block {block:016x?}?) in thread registers")
            }
            
            // scan thread stacks
            let bounds = get_thread_stack_bounds(thread).unwrap();
            let stack_ptr = bounds.0.with_addr(context.Rsp as usize) as *const ();
            for ptr in unsafe { scan_stack(contains, bounds, stack_ptr) } {
                let block = ptr.wrapping_byte_offset(-0x20);
                info!("Found pointer to {ptr:016x?} (maybe block {block:016x?}?) in thread stack")
            }
        }
        
        debug!("finished scanning");
    }
}
