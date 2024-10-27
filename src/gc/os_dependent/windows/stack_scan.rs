use windows_sys::Win32::Foundation::NTSTATUS;

use super::thread::{map_other_threads, get_thread_teb};

/// Get the upper and lower limits for the stack memory for a given thread.
pub(super) fn get_thread_stack_bounds(thread_handle: windows_sys::Win32::Foundation::HANDLE) -> Result<(*const core::ffi::c_void, *const core::ffi::c_void), NTSTATUS> {
    let teb = get_thread_teb(thread_handle)?;
    Ok(unsafe { ((*teb).tib.stack_limit, (*teb).tib.stack_base) })
}




/// returns all scannable stack memory in the current process.
pub fn get_all_thread_stack_bounds() -> Vec<(*const core::ffi::c_void, *const core::ffi::c_void)> {
    let mut result = Vec::new();
    map_other_threads(|handle| result.push(get_thread_stack_bounds(handle).unwrap())).unwrap();
    result
}


