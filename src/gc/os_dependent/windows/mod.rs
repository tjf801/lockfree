mod stack_scan;
mod heap_scan;
mod thread;
pub mod mem_source;


use std::mem::MaybeUninit;

pub use stack_scan::get_all_thread_stack_bounds;
use thread::map_other_threads;
use windows_sys::Win32::System::Diagnostics::Debug::CONTEXT;

pub fn flush_process_write_buffers() {
    use windows_sys::Win32::System::Threading::FlushProcessWriteBuffers;
    // TODO: this combined with volatile reads is enough for memory scanning on
    //       x86, but is it portable to ARM windows?? also, is this even
    //       POTENTIALLY a race? the documentation for this function¹ says that
    //       "It guarantees the visibility of write operations performed on one
    //       processor to the other processors", but that doesnt say what kind
    //       of read you need for that. Obviously a `SeqCst` would be enough,
    //       but it makes no mention of atomics, so i would *assume* non-atomic
    //       reads are fine too..? i honestly have no idea at the moment.
    // ¹: https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-flushprocesswritebuffers
    unsafe { FlushProcessWriteBuffers() };
}

#[repr(align(16))]
#[derive(Clone, Copy)]
pub struct Align16<T: ?Sized>(T);
#[cfg(target_arch="x86_64")]
impl std::fmt::Debug for Align16<CONTEXT> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CONTEXT")
            .field("rip", &self.0.Rip)
            .field("FS", &self.0.SegFs)
            .field("GS", &self.0.SegGs)
            .field("rax", &self.0.Rax)
            .field("rbx", &self.0.Rbx)
            .field("rcx", &self.0.Rcx)
            .field("rdx", &self.0.Rdx)
            .field("rsi", &self.0.Rsi)
            .field("rdi", &self.0.Rdi)
            .field("rsp", &self.0.Rsp)
            .field("rbp", &self.0.Rbp)
            .field("r8", &self.0.R8)
            .field("r9", &self.0.R9)
            .field("r10", &self.0.R10)
            .field("r11", &self.0.R11)
            .field("r12", &self.0.R12)
            .field("r13", &self.0.R13)
            .field("r14", &self.0.R14)
            .field("r15", &self.0.R15)
        .finish_non_exhaustive()
    }
}

pub fn get_thread_context(thread_handle: *mut std::ffi::c_void) -> Result<Align16<CONTEXT>, ()> {
    use windows_sys::Win32::System::Diagnostics::Debug::{InitializeContext, GetThreadContext};
    use windows_sys::Win32::Foundation::GetLastError;
    #[allow(unused_imports)]
    use windows_sys::Win32::System::Diagnostics::Debug::{CONTEXT_ALL_AMD64, CONTEXT_ALL_X86, CONTEXT_ALL_ARM, CONTEXT_ALL_ARM64};
    
    #[cfg(target_arch="x86_64")] let context_flags = CONTEXT_ALL_AMD64;
    #[cfg(target_arch="x86")] let context_flags = CONTEXT_ALL_X86;
    #[cfg(target_arch="arm")] let context_flags = CONTEXT_ALL_ARM;
    #[cfg(target_arch="aarch64")] let context_flags = CONTEXT_ALL_ARM64;
    
    let mut length: u32 = 0;
    let rv = unsafe { InitializeContext(std::ptr::null_mut(), context_flags, std::ptr::null_mut(), &raw mut length) };
    if rv == 0 {
        let err = unsafe { GetLastError() };
        if err != windows_sys::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER {
            error!("InitializeContext failed with code {err:x}");
            return Err(())
        }
    } else {
        unreachable!("calling `InitializeContext` with a null pointer will never succeed")
    }
    
    let mut context: MaybeUninit<Align16<CONTEXT>> = MaybeUninit::uninit();
    
    // TODO: why does this assertion fail? that seems bad.
    if size_of_val(&context) >= length as usize {
        warn!("Size of context buffer ({} bytes) is smaller than required ({length} bytes)", size_of_val(&context));
        // assert!(size_of_val(&context) >= length as usize, "{} < {length}", size_of_val(&context));
    }
    
    let mut _context_ptr = std::ptr::null_mut();
    let rv = unsafe { InitializeContext(context.as_mut_ptr() as _, context_flags, &raw mut _context_ptr, &raw mut length) };
    if rv == 0 {
        let err = unsafe { GetLastError() };
        error!("InitializeContext failed with code {err:x}");
        return Err(())
    }
    
    assert_eq!(_context_ptr, context.as_mut_ptr() as _);
    
    let rv = unsafe { GetThreadContext(thread_handle, &raw mut context as _) };
    if rv == 0 {
        let err = unsafe { GetLastError() };
        error!("GetThreadContext failed with code {err:x}");
        return Err(())
    }
    
    Ok(unsafe { context.assume_init() })
}

/// pauses the execution of all other threads
pub fn stop_the_world() {
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::System::Threading::{GetThreadId, SuspendThread};
    
    // NOTE: doing this does not create deadlocks that weren't already there.
    //       The OS can suspend and resume threads at any time however it likes,
    //       and we are just doing that
    map_other_threads(|thread_handle| {
        // TODO: do this synchronously somehow
        //  - https://devblogs.microsoft.com/oldnewthing/20150205-00/?p=44743
        //  - https://stackoverflow.com/questions/5720326/suspending-and-resuming-threads-in-c
        //  - https://osm.hpi.de/wrk/2009/01/what-does-suspendthread-really-do/
        if unsafe { SuspendThread(thread_handle) } == u32::MAX {
            // TODO: why does this happen??? and only very inconsistently?
            match unsafe { GetLastError() } {
                0x05 => trace!("access denied to thread 0x{:x}", unsafe { GetThreadId(thread_handle) }),
                error => panic!("couldnt suspend thread (error code 0x{:x}): HANDLE {thread_handle:016x?}", error)
            }
        }
    }).unwrap()
}


/// resumes the execution of all other threads
pub fn start_the_world() {
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::System::Threading::ResumeThread;
    map_other_threads(|thread_handle| {
        if unsafe { ResumeThread(thread_handle) } == u32::MAX {
            error!("couldnt resume thread (error code 0x{:x})", unsafe { GetLastError() });
        }
    }).unwrap()
}


#[cfg(test)]
mod tests {
    use std::time::Duration;

    use stack_scan::get_all_thread_stack_bounds;

    use super::*;
    
    // just some unoptimizable busywork for test threads to do
    fn partitions_recursive(n: u64) -> u64 {
        if n == 0 { return 1 }
        if n <= 3 { return n }
        fn pent(n: i64) -> u64 {
            (n*(3*n-1)/2).try_into().unwrap()
        }
        let mut i = 1;
        let mut sum = 0;
        while pent(-2*i) <= n {
            sum += partitions_recursive(n - pent(2*i-1));
            sum += partitions_recursive(n - pent(-(2*i-1)));
            sum -= partitions_recursive(n - pent(2*i));
            sum -= partitions_recursive(n - pent(-2*i));
            i += 1;
        }
        if pent(2*i-1) <= n { sum += partitions_recursive(n - pent(2*i-1)) }
        if pent(-(2*i-1)) <= n { sum += partitions_recursive(n - pent(-(2*i-1))) }
        if pent(2*i) <= n { sum -= partitions_recursive(n - pent(2*i)) }
        assert!(pent(-2*i) > n);
        sum
    }
    
    #[test]
    fn test_thread_suspend_resume() {
        for i in 0..10 {
            let _ = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(100*i));
                println!("{i}");
            });
        }
        std::thread::sleep(Duration::from_millis(99));
        for bounds in get_all_thread_stack_bounds() {
            println!("{bounds:?}")
        }
        let start = std::time::Instant::now();
        for _ in 0..1000 {
            stop_the_world();
            start_the_world();
        }
        println!("time: {:?}", std::time::Instant::now() - start);
    }
    
    #[test]
    fn test_thread_context() {
        fn thread_work(i: u64) {
            let x = [2, 13, 24, 31, 46, 65, 79, 100, 245, 486][i as usize];
            println!("Starting thread {i} on value {x}");
            let c = partitions_recursive(x);
            println!("Thread {i}: {c}");
        }
        struct Thing;
        impl Thing { fn new() -> Self { stop_the_world(); Thing } }
        impl Drop for Thing { fn drop(&mut self) { start_the_world(); } }
        for i in 0..10 {
            let _ = std::thread::spawn(move || thread_work(i));
        }
        std::thread::sleep_ms(10);
        let t = Thing::new();
        map_other_threads(|handle| {
            let c = get_thread_context(handle);
            let x = stack_scan::get_thread_stack_bounds(handle);
            println!("{handle:08x?} {x:x?}");
            println!("{c:x?}");
        }).unwrap();
        std::hint::black_box(t);
    }
}
