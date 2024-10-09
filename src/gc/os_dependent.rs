use std::mem::MaybeUninit;

#[cfg(target_os="windows")]
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

#[cfg(target_os="windows")]
#[repr(C)]
struct ThreadEnvironmentBlock {
    tib: ThreadInformationBlock,
    environment_pointer: *const core::ffi::c_void,
    
}

#[cfg(target_os="windows")]
fn get_thread_teb(thread_handle: windows_sys::Win32::Foundation::HANDLE) -> *const ThreadEnvironmentBlock {
    use windows_sys::Wdk::System::Threading::{NtQueryInformationThread, ThreadBasicInformation};
    use windows_sys::Win32::Data::HtmlHelp::PRIORITY;
    use windows_sys::Win32::Foundation::{GetLastError, NTSTATUS};
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
    if rv != 0 { panic!("{rv:x} {:x}", unsafe { GetLastError() }) }
    
    let buffer = unsafe { buffer.assume_init() };
    
    buffer.teb_base_address
}

#[cfg(all(target_os="windows"))]
pub fn get_thread_stack_bounds(thread_handle: windows_sys::Win32::Foundation::HANDLE) -> (*const core::ffi::c_void, *const core::ffi::c_void) {
    let teb = get_thread_teb(thread_handle);
    unsafe { ((*teb).tib.stack_limit, (*teb).tib.stack_base) }
}

/// a re-export of [`GetCurrentThreadId`]
/// 
/// [`GetCurrentThreadId`]: windows_sys::Win32::System::Threading::GetCurrentThreadId
#[cfg(target_os="windows")]
fn get_current_thread_id() -> u32 {
    unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() }
}

#[cfg(target_os="windows")]
fn get_other_thread_handles() -> Result<Vec<windows_sys::Win32::Foundation::HANDLE>, ()> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Threading::{GetCurrentProcessId, OpenThread, THREAD_ALL_ACCESS};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPALL, THREADENTRY32};
    
    let process_id = unsafe { GetCurrentProcessId() };
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPALL, process_id) };
    if snapshot == INVALID_HANDLE_VALUE { return Err(()) }
    
    let this_thread_id = get_current_thread_id();
    
    let mut thread_entry = THREADENTRY32 {
        dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
        cntUsage: 0,
        th32ThreadID: 0,
        th32OwnerProcessID: 0,
        tpBasePri: 0,
        tpDeltaPri: 0,
        dwFlags: 0
    };
    
    if unsafe { Thread32First(snapshot, &raw mut thread_entry) } == 0 {
        unsafe { CloseHandle(snapshot) };
        return Err(());
    }
    
    let mut handles = Vec::new();
    
    while unsafe { Thread32Next(snapshot, &raw mut thread_entry) } != 0 {
        if thread_entry.th32OwnerProcessID == process_id {
            if thread_entry.th32ThreadID == this_thread_id { continue }
            let handle = unsafe { OpenThread(THREAD_ALL_ACCESS, 1, thread_entry.th32ThreadID) };
            handles.push(handle);
        }
    }
    
    unsafe { CloseHandle(snapshot) };
    Ok(handles)
}

#[cfg(target_os="windows")]
pub fn stop_the_world() -> Result<(), ()> {
    use windows_sys::Win32::System::Threading::SuspendThread;
    
    for thread_handle in get_other_thread_handles()? {
        if unsafe { SuspendThread(thread_handle) } == u32::MAX {
            return Err(())
        }
    }
    
    Ok(())
}

#[cfg(target_os="windows")]
pub fn start_the_world() -> Result<(), ()> {
    use windows_sys::Win32::System::Threading::ResumeThread;
    
    for thread_handle in get_other_thread_handles()? {
        if unsafe { ResumeThread(thread_handle) } == u32::MAX {
            // println!("{}", unsafe { GetLastError() });
            return Err(())
        }
    }
    
    Ok(())
}



#[test]
fn test() {
    for i in 0..10 {
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100 * i));
            println!("{i}");
        });
    }
    
    stop_the_world().unwrap();
    
    std::thread::sleep_ms(1500);
    
    start_the_world().unwrap();
    
    panic!();
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test() {
        for handle in get_other_thread_handles().unwrap() {
            println!("{:?}", get_thread_stack_bounds(handle));
        }
    }
}
