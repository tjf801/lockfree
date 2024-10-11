use std::mem::MaybeUninit;

use windows_sys::Win32::Foundation::{HANDLE, NTSTATUS, STATUS_NO_MORE_ENTRIES};
use windows_sys::Win32::System::Threading::{GetCurrentThreadId, GetThreadId};

// TODO: heap scan using
// https://learn.microsoft.com/en-us/windows/win32/api/heapapi/nf-heapapi-heaplock
// https://learn.microsoft.com/en-us/windows/win32/api/heapapi/nf-heapapi-heapwalk

#[repr(C)]
struct ThreadInformationBlock {
    exception_list: *const core::ffi::c_void,
    stack_base: *const core::ffi::c_void,
    stack_limit: *const core::ffi::c_void,
    subsystem_tib: *const core::ffi::c_void,
    thing: usize, // bruh i hate unions
    arbitrary_user_pointer: *const core::ffi::c_void,
    _self: *const ThreadInformationBlock, // me when pin
}


#[repr(C)]
struct ThreadEnvironmentBlock {
    tib: ThreadInformationBlock,
    environment_pointer: *const core::ffi::c_void,
    // ... (dont care)
}


/// Given a handle to a thread, return a pointer to the thread's [TEB](https://en.wikipedia.org/wiki/Win32_Thread_Information_Block).
fn get_thread_teb(thread_handle: windows_sys::Win32::Foundation::HANDLE) -> Result<*const ThreadEnvironmentBlock, NTSTATUS> {
    use windows_sys::Wdk::System::Threading::{NtQueryInformationThread, ThreadBasicInformation};
    use windows_sys::Win32::Data::HtmlHelp::PRIORITY;
    use windows_sys::Win32::Foundation::NTSTATUS;
    use windows_sys::Win32::System::WindowsProgramming::CLIENT_ID;
    
    #[repr(C)]
    struct _ThreadBasicInformation {
        exit_status: NTSTATUS,
        teb_base_address: *const ThreadEnvironmentBlock,
        client_id: CLIENT_ID,
        affinity_mask: core::ffi::c_ulong,
        priority: PRIORITY,
        base_priority: PRIORITY,
    }
    
    let mut return_length: core::ffi::c_ulong = core::ffi::c_ulong::MAX;
    let mut buffer: std::mem::MaybeUninit<_ThreadBasicInformation> = MaybeUninit::uninit();
    
    let rv = unsafe {
        NtQueryInformationThread(
            thread_handle,
            ThreadBasicInformation,
            &raw mut buffer as _,
            std::mem::size_of_val_raw(&raw const buffer) as core::ffi::c_ulong,
            &raw mut return_length
        )
    };
    if rv != 0 { return Err(rv) }
    
    let buffer = unsafe { buffer.assume_init() };
    
    Ok(buffer.teb_base_address)
}


/// Get the upper and lower limits for the stack memory for a given thread.
fn get_thread_stack_bounds(thread_handle: windows_sys::Win32::Foundation::HANDLE) -> Result<(*const core::ffi::c_void, *const core::ffi::c_void), NTSTATUS> {
    let teb = get_thread_teb(thread_handle)?;
    Ok(unsafe { ((*teb).tib.stack_limit, (*teb).tib.stack_base) })
}


/// a re-export of [`GetCurrentThreadId`]
/// 
/// [`GetCurrentThreadId`]: windows_sys::Win32::System::Threading::GetCurrentThreadId
fn get_current_thread_id() -> u32 {
    unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() }
}


#[link(name = "ntdll.dll", kind = "raw-dylib", modifiers = "+verbatim")]
unsafe extern "system" {
    pub fn NtGetNextThread(
        ProcessHandle: HANDLE,
        ThreadHandle: HANDLE,
        DesiredAccess: u32,
        HandleAttributes: u32,
        Flags: u32,
        NewThreadHandle: *mut HANDLE,
    ) -> NTSTATUS;
}


/// Finds all other thread handles associated with the current process.
// thanks to:
// https://ntdoc.m417z.com/ntgetnextthread
// https://stackoverflow.com/questions/61870414/is-there-a-fast-way-to-list-the-threads-in-the-current-windows-process
fn map_other_threads(mut func: impl FnMut(HANDLE) -> ()) -> Result<(), NTSTATUS> {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, THREAD_ALL_ACCESS};
    
    let current_thread_id = unsafe { GetCurrentThreadId() };
    let current_process_handle = unsafe { GetCurrentProcess() };
    
    let mut thread_handle: HANDLE = std::ptr::null_mut();    
    loop {
        let mut next_thread_handle: HANDLE = std::ptr::null_mut();
        
        let status = unsafe { NtGetNextThread(current_process_handle, thread_handle, THREAD_ALL_ACCESS, 0, 0, &raw mut next_thread_handle) };
        
        if status == STATUS_NO_MORE_ENTRIES { break }
        if status != 0 { return Err(status) }
        
        if thread_handle != std::ptr::null_mut() && unsafe { CloseHandle(thread_handle) } == 0 {
            return Err(unsafe { GetLastError() } as i32)
        }
        
        thread_handle = next_thread_handle;
        
        if unsafe { GetThreadId(thread_handle) } != current_thread_id {
            func(thread_handle);
        }
    }
    
    if unsafe { CloseHandle(thread_handle) } == 0 {
        return Err(unsafe { GetLastError() } as i32)
    }
    
    Ok(())
}


/// returns all scannable stack memory in the current process.
pub fn get_all_thread_stack_bounds() -> Vec<(*const core::ffi::c_void, *const core::ffi::c_void)> {
    let mut result = Vec::new();
    map_other_threads(|handle| result.push(get_thread_stack_bounds(handle).unwrap())).unwrap();
    result
}


/// pauses the execution of all other threads
pub fn stop_the_world() {
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::System::Threading::SuspendThread;
    
    // NOTE: doing this does not create deadlocks that weren't already there.
    //       The OS can suspend and resume threads at any time however it likes,
    //       and we are just doing that
    map_other_threads(|thread_handle| {
        if unsafe { SuspendThread(thread_handle) } == u32::MAX {
            // let thread_id = unsafe { GetThreadId(thread_handle) };
            panic!("couldnt suspend thread (error code 0x{:x}): HANDLE {thread_handle:016x?}", unsafe { GetLastError() });
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
        let start = std::time::Instant::now();
        stop_the_world();
        for bounds in get_all_thread_stack_bounds() {
            println!("{bounds:?}")
        }
        start_the_world();
        let time = std::time::Instant::now() - start;
        println!("time: {time:?}");
    }
}
