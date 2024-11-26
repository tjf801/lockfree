use windows_sys::Win32::Foundation::NTSTATUS;

use super::thread::get_thread_teb;

/// Get the upper and lower limits for the stack memory for a given thread.
pub fn get_thread_stack_bounds(thread_handle: windows_sys::Win32::Foundation::HANDLE) -> Result<(*const (), *const ()), NTSTATUS> {
    let teb = get_thread_teb(thread_handle)?;
    Ok(unsafe { ((*teb).tib.stack_limit as _, (*teb).tib.stack_base as _) })
}
