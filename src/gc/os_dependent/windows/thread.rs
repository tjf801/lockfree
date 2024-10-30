use std::mem::MaybeUninit;

use windows_sys::Win32::Foundation::{HANDLE, NTSTATUS};


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

/// Gets all (other) thread handles associated with the current process.
// thanks to:
// https://ntdoc.m417z.com/ntgetnextthread
// https://stackoverflow.com/questions/61870414/is-there-a-fast-way-to-list-the-threads-in-the-current-windows-process
pub fn get_all_threads() -> impl IntoIterator<Item=Result<HANDLE, NTSTATUS>> {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, STATUS_NO_MORE_ENTRIES};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentThreadId, GetThreadId, THREAD_ALL_ACCESS};
    
    gen {
        let current_thread_id = unsafe { GetCurrentThreadId() };
        let current_process_handle = unsafe { GetCurrentProcess() };
        
        let mut current_thread_handle: HANDLE = std::ptr::null_mut();
        loop {
            let mut next_thread_handle = std::ptr::null_mut();
            
            let status = unsafe { NtGetNextThread(current_process_handle, current_thread_handle, THREAD_ALL_ACCESS, 0, 0, &raw mut next_thread_handle) };
            
            if status == STATUS_NO_MORE_ENTRIES { break }
            if status != 0 { yield Err(status) }
            
            if !current_thread_handle.is_null() && unsafe { CloseHandle(current_thread_handle) } == 0 {
                warn!("Error in `CloseHandle({current_thread_handle:x?})`, code ({:016x})", unsafe { GetLastError() });
            }
            
            current_thread_handle = next_thread_handle;
            
            if unsafe { GetThreadId(current_thread_handle) } != current_thread_id {
                yield Ok(current_thread_handle);
            }
        }
        
        if unsafe { CloseHandle(current_thread_handle) } == 0 {
            warn!("Error in `CloseHandle({current_thread_handle:x?})`, code ({:016x})", unsafe { GetLastError() });
        }
    }
}


#[repr(C)]
pub struct ThreadInformationBlock {
    pub exception_list: *const core::ffi::c_void,
    pub stack_base: *const core::ffi::c_void,
    pub stack_limit: *const core::ffi::c_void,
    pub subsystem_tib: *const core::ffi::c_void,
    pub thing: usize, // bruh i hate unions
    pub arbitrary_user_pointer: *const core::ffi::c_void,
    pub _self: *const ThreadInformationBlock, // me when pin
}


/// https://ntdoc.m417z.com/teb
#[repr(C)]
pub struct ThreadEnvironmentBlock {
    pub tib: ThreadInformationBlock,
    pub environment_pointer: *const core::ffi::c_void,
    // ... (dont care)
}


/// Given a handle to a thread, return a pointer to the thread's [TEB](https://en.wikipedia.org/wiki/Win32_Thread_Information_Block).
pub fn get_thread_teb(thread_handle: windows_sys::Win32::Foundation::HANDLE) -> Result<*const ThreadEnvironmentBlock, NTSTATUS> {
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
    
    let buffer_init = unsafe { buffer.assume_init() };
    
    if buffer_init.teb_base_address == std::ptr::null() {
        error!("Thread (id: {:x}, handle: {:x?}) had null TEB", unsafe {windows_sys::Win32::System::Threading::GetThreadId(thread_handle)}, thread_handle);
        return Err(0)
    }
    
    Ok(buffer_init.teb_base_address)
}
