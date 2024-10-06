#[cfg(target_os="windows")]
pub fn get_current_stack_bounds() -> (usize, usize) {
    use windows_sys::Win32::System::Threading::GetCurrentThreadStackLimits;
    
    let mut lowlimit: usize = 0;
    let mut highlimit: usize = 0;
    
    // SAFETY: lowlimit and highlimit are okay to write to
    unsafe { GetCurrentThreadStackLimits(&raw mut lowlimit, &raw mut highlimit) }; // omfg i LOVE the new raw ref syntax. i didnt know how much i needed it
    
    debug_assert!(lowlimit <= highlimit);
    
    (lowlimit, highlimit)
}

#[cfg(target_os="windows")]
pub fn get_current_thread_real_handle() -> Result<windows_sys::Win32::Foundation::HANDLE, windows_sys::Win32::Foundation::WIN32_ERROR> {
    use windows_sys::Win32::Foundation::{DuplicateHandle, GetLastError, DUPLICATE_SAME_ACCESS};
    
    // NOTE: [this] cannot be used by one thread to create a handle that can be
    // used by other threads to refer to the first thread. The handle is always
    // interpreted as referring to the thread that is using it. A thread can
    // create a "real" handle to itself that can be used by other threads, or
    // inherited by other processes, by specifying the pseudo handle as the
    // source handle in a call to the `DuplicateHandle` function.
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentThread};
    
    let pseudo_handle = unsafe { GetCurrentThread() };
    let process_handle = unsafe { GetCurrentProcess() };
    
    let mut out_handle = std::ptr::null_mut();
    
    // SAFETY: no clue what the preconditions are for this function, but out_handle is writable, and this seems good to me
    let rv = unsafe { DuplicateHandle(process_handle, pseudo_handle, 0 as _, &raw mut out_handle, 0, 0, DUPLICATE_SAME_ACCESS) };
    if rv != 0 { return Err(unsafe { GetLastError() }) }
    
    Ok(out_handle)
}

#[cfg(target_os="windows")]
pub fn get_thread_stack_bounds(thread_handle: windows_sys::Win32::Foundation::HANDLE) -> (usize, usize) {
    
    
    todo!()
}

/// a re-export of [`GetCurrentThreadId`]
/// 
/// [`GetCurrentThreadId`]: windows_sys::Win32::System::Threading::GetCurrentThreadId
#[cfg(target_os="windows")]
pub fn get_current_thread_id() -> u32 {
    unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() }
}

#[cfg(target_os="windows")]
pub fn get_other_thread_handles() -> Result<Vec<windows_sys::Win32::Foundation::HANDLE>, ()> {
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
            // println!("[{}]: owner pid: {}", thread_entry.th32ThreadID, thread_entry.th32OwnerProcessID);
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
        
    }
}
