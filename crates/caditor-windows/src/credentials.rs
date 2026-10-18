use std::{io, ptr, slice};

use windows_sys::Win32::{
    Foundation::ERROR_NOT_FOUND,
    Security::Credentials::{
        CRED_MAX_CREDENTIAL_BLOB_SIZE, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW,
        CredDeleteW, CredFree, CredReadW, CredWriteW,
    },
};

use crate::files::wide;

pub const CREDENTIAL_BLOB_LIMIT: usize = CRED_MAX_CREDENTIAL_BLOB_SIZE as usize;

const NO_FLAGS: u32 = 0;

pub fn read_credential(target: &str) -> io::Result<Option<Vec<u8>>> {
    let target = wide(target);
    let mut found: *mut CREDENTIALW = ptr::null_mut();
    #[expect(unsafe_code, reason = "the Win32 call is unsafe to make")]
    let read = unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, NO_FLAGS, &raw mut found) };
    if read == 0 {
        return not_found_as_none(io::Error::last_os_error());
    }
    let credential = Found(found);
    Ok(Some(credential.blob()))
}

pub fn write_credential(target: &str, user: &str, blob: &[u8]) -> io::Result<()> {
    let size = u32::try_from(blob.len())
        .ok()
        .filter(|size| *size <= CRED_MAX_CREDENTIAL_BLOB_SIZE)
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    let mut target = wide(target);
    let mut user = wide(user);
    let mut blob = blob.to_vec();
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: target.as_mut_ptr(),
        CredentialBlobSize: size,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        UserName: user.as_mut_ptr(),
        ..CREDENTIALW::default()
    };
    #[expect(unsafe_code, reason = "the Win32 call is unsafe to make")]
    let written = unsafe { CredWriteW(&raw const credential, NO_FLAGS) };
    if written == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub fn delete_credential(target: &str) -> io::Result<bool> {
    let target = wide(target);
    #[expect(unsafe_code, reason = "the Win32 call is unsafe to make")]
    let deleted = unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, NO_FLAGS) };
    if deleted == 0 {
        return not_found_as_none(io::Error::last_os_error()).map(|_: Option<()>| false);
    }
    Ok(true)
}

fn not_found_as_none<T>(error: io::Error) -> io::Result<Option<T>> {
    if error.raw_os_error() == i32::try_from(ERROR_NOT_FOUND).ok() {
        Ok(None)
    } else {
        Err(error)
    }
}

struct Found(*mut CREDENTIALW);

impl Found {
    fn blob(&self) -> Vec<u8> {
        #[expect(
            unsafe_code,
            reason = "CredReadW returned a credential this guard owns"
        )]
        let credential = unsafe { &*self.0 };
        if credential.CredentialBlob.is_null() {
            return Vec::new();
        }
        let size = usize::try_from(credential.CredentialBlobSize).unwrap_or(0);
        #[expect(
            unsafe_code,
            reason = "the credential's blob holds CredentialBlobSize readable bytes"
        )]
        unsafe { slice::from_raw_parts(credential.CredentialBlob, size) }.to_vec()
    }
}

impl Drop for Found {
    fn drop(&mut self) {
        #[expect(unsafe_code, reason = "the Win32 call is unsafe to make")]
        unsafe {
            CredFree(self.0.cast());
        }
    }
}
