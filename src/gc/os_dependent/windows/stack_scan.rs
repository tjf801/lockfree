use windows_sys::Win32::Foundation::NTSTATUS;

use super::thread::{get_all_threads, get_thread_teb};

/// Get the upper and lower limits for the stack memory for a given thread.
pub fn get_thread_stack_bounds(thread_handle: windows_sys::Win32::Foundation::HANDLE) -> Result<(*const (), *const ()), NTSTATUS> {
    let teb = get_thread_teb(thread_handle)?;
    Ok(unsafe { ((*teb).tib.stack_limit as _, (*teb).tib.stack_base as _) })
}




/// returns all scannable stack memory in the current process.
pub fn get_all_thread_stack_bounds() -> impl Iterator<Item=(*const (), *const ())> {
    get_all_threads().into_iter().map(Result::unwrap).map(get_thread_stack_bounds).map(Result::unwrap)
}


