use std::ptr::NonNull;

use super::super::{MEMORY_SOURCE, MemorySource};
use super::super::heap_block_header::GCHeapBlockHeader;
use super::super::os_dependent::windows::heap_scan::WinHeapLock;

pub(super) fn scan_registers(c: &windows_sys::Win32::System::Diagnostics::Debug::CONTEXT) -> impl IntoIterator<Item=*const ()> {
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

pub(super) unsafe fn scan_stack(bounds: (*const (), *const ()), rsp: *const ()) -> impl IntoIterator<Item=*const ()> {
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

pub(super) unsafe fn scan_segment(data: NonNull<[u8]>) -> impl IntoIterator<Item=*const ()> {
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

pub(super) fn scan_heap(roots: &mut Vec<*const ()>, mut lock: WinHeapLock) {
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

pub(super) fn scan_block(block: &GCHeapBlockHeader) -> impl IntoIterator<Item=*const ()> {
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

