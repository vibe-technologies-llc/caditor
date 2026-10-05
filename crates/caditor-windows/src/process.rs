use std::io;

use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_ACCESS_DENIED, HANDLE, WAIT_TIMEOUT},
    System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    },
};

const NO_WAIT: u32 = 0;
const NOT_INHERITED: i32 = 0;

pub fn process_running(process: u32) -> bool {
    #[allow(unsafe_code)]
    let handle = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            NOT_INHERITED,
            process,
        )
    };
    if handle.is_null() {
        return io::Error::last_os_error().raw_os_error()
            == i32::try_from(ERROR_ACCESS_DENIED).ok();
    }
    let process = Process(handle);
    #[allow(unsafe_code)]
    let waited = unsafe { WaitForSingleObject(process.0, NO_WAIT) };
    waited == WAIT_TIMEOUT
}

struct Process(HANDLE);

impl Drop for Process {
    fn drop(&mut self) {
        #[allow(unsafe_code)]
        unsafe {
            CloseHandle(self.0);
        }
    }
}
