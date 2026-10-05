use std::ptr;

use windows_sys::Win32::{
    Foundation::ERROR_SUCCESS,
    System::Registry::{
        HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6464KEY, RegGetValueW,
    },
};

use crate::files::wide;

const CRYPTOGRAPHY: &str = r"SOFTWARE\Microsoft\Cryptography";
const MACHINE_GUID: &str = "MachineGuid";
const PREFETCH: &str =
    r"SYSTEM\CurrentControlSet\Control\Session Manager\Memory Management\PrefetchParameters";
const BOOT_ID: &str = "BootId";
const GUID_UNITS: usize = 64;

pub fn machine_guid() -> Option<String> {
    let key = wide(CRYPTOGRAPHY);
    let value = wide(MACHINE_GUID);
    let mut buffer = [0u16; GUID_UNITS];
    let mut size = u32::try_from(size_of_val(&buffer)).ok()?;
    #[allow(unsafe_code)]
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ | RRF_SUBKEY_WOW6464KEY,
            ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &raw mut size,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let units = usize::try_from(size).ok()? / size_of::<u16>();
    let text = buffer.get(..units)?;
    let end = text
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(text.len());
    String::from_utf16(text.get(..end)?).ok()
}

pub fn boot_id() -> Option<u32> {
    let key = wide(PREFETCH);
    let value = wide(BOOT_ID);
    let mut boot = 0u32;
    let mut size = u32::try_from(size_of::<u32>()).ok()?;
    #[allow(unsafe_code)]
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD | RRF_SUBKEY_WOW6464KEY,
            ptr::null_mut(),
            (&raw mut boot).cast(),
            &raw mut size,
        )
    };
    (status == ERROR_SUCCESS).then_some(boot)
}
