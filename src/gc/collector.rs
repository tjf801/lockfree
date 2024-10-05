use std::arch::asm;

#[cfg(target_os="windows")]
fn get_current_stack_bounds() -> (usize, usize) {
    use windows_sys::Win32::System::Threading::GetCurrentThreadStackLimits;
    
    let mut lowlimit: usize = 0;
    let mut highlimit: usize = 0;
    
    /// SAFETY: lowlimit and highlimit are okay to write to
    unsafe { GetCurrentThreadStackLimits(&raw mut lowlimit, &raw mut highlimit) }; // omfg i LOVE the new raw ref syntax. i didnt know how much i needed it
    
    debug_assert!(lowlimit <= highlimit);
    
    (lowlimit, highlimit)
}

#[cfg(target_os="windows")]
fn get_current_thread_real_handle() -> Result<windows_sys::Win32::Foundation::HANDLE, windows_sys::Win32::Foundation::WIN32_ERROR> {
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
    
    /// SAFETY: no clue what the preconditions are for this function, but out_handle is writable, and this seems good to me
    let rv = unsafe { DuplicateHandle(process_handle, pseudo_handle, 0 as _, &raw mut out_handle, 0, 0, DUPLICATE_SAME_ACCESS) };
    if rv != 0 { return Err(unsafe { GetLastError() }) }
    
    Ok(out_handle)
}

#[cfg(target_os="windows")]
fn get_thread_stack_bounds(thread_handle: windows_sys::Win32::Foundation::HANDLE) -> (usize, usize) {
    
    
    todo!()
}

#[cfg(target_os="windows")]
fn stop_the_world() {
    use windows_sys::Win32::System::Threading::SuspendThread;
    
    todo!()
}



#[test]
fn test() {
    
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test() {
        
    }
}
