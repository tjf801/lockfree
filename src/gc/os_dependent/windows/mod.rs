mod stack_scan;
mod heap_scan;
mod thread;


pub use stack_scan::get_all_thread_stack_bounds;
use thread::map_other_threads;


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
                0x05 => println!("access denied to thread 0x{:x} :(", unsafe { GetThreadId(thread_handle) }),
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
            println!("couldnt resume thread (error code 0x{:x})", unsafe { GetLastError() });
        }
    }).unwrap()
}


#[cfg(test)]
mod tests {
    use std::time::Duration;

    use stack_scan::get_all_thread_stack_bounds;

    use super::*;
    
    #[test]
    fn test() {
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
}
