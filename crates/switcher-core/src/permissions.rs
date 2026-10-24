//! Private state-directory ACLs. This changes only the tool-owned directory.
use crate::{Error, Result};
use std::{os::windows::ffi::OsStrExt, path::Path, ptr};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, LocalFree},
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SE_FILE_OBJECT, SetNamedSecurityInfoW,
        },
        DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl, GetTokenInformation,
        PROTECTED_DACL_SECURITY_INFORMATION, TOKEN_QUERY, TOKEN_USER, TokenUser,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

struct Token(HANDLE);
impl Drop for Token {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
struct Local(*mut core::ffi::c_void);
impl Drop for Local {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}

/// Grant the calling user and SYSTEM inheritable access, excluding inherited grants.
/// All Windows allocations and token handles are released on every error path.
pub fn restrict_directory(path: &Path) -> Result<()> {
    let error =
        || Error("Cannot restrict local state permissions. Use a private NTFS user directory.");
    // Buffers remain alive through each Windows call; the token buffer is pointer-aligned.
    unsafe {
        let mut handle = ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle) == 0 {
            return Err(error());
        }
        let token = Token(handle);
        let mut required = 0;
        GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut required);
        if required == 0 || required > 65536 {
            return Err(error());
        }
        let mut buffer = vec![0_usize; (required as usize).div_ceil(size_of::<usize>())];
        if GetTokenInformation(
            token.0,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            required,
            &mut required,
        ) == 0
        {
            return Err(error());
        }
        let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
        let mut sid = ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut sid) == 0 {
            return Err(error());
        }
        let _sid_allocation = Local(sid.cast());
        let mut length = 0;
        while length < 256 && *sid.add(length) != 0 {
            length += 1;
        }
        if length == 256 {
            return Err(error());
        }
        let sid =
            String::from_utf16(std::slice::from_raw_parts(sid, length)).map_err(|_| error())?;
        let descriptor: Vec<u16> = format!("D:P(A;OICI;FA;;;{sid})(A;OICI;FA;;;SY)")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut security = ptr::null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            descriptor.as_ptr(),
            1,
            &mut security,
            ptr::null_mut(),
        ) == 0
        {
            return Err(error());
        }
        let _security_allocation = Local(security);
        let mut present = 0;
        let mut defaulted = 0;
        let mut acl = ptr::null_mut();
        if GetSecurityDescriptorDacl(security, &mut present, &mut acl, &mut defaulted) == 0
            || present == 0
            || acl.is_null()
        {
            return Err(error());
        }
        let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        if SetNamedSecurityInfoW(
            path.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            acl,
            ptr::null_mut(),
        ) != 0
        {
            return Err(error());
        }
    }
    Ok(())
}
