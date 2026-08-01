use std::{fs::OpenOptions, io, path::Path};

/// Apply the platform's safest available flags before opening a private
/// application file. On Unix this prevents following a final-component
/// symlink and prevents descriptor inheritance. On Windows it opens a reparse
/// point itself so the post-open validation can reject it instead of silently
/// following it.
pub(crate) fn configure_private_open(options: &mut OpenOptions, _unix_mode: u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(_unix_mode)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;

        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
}

pub(crate) fn harden_private_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(target_os = "windows")]
    {
        validate_windows_path(path, true)?;
        apply_windows_private_acl(path, true)?;
    }
    #[cfg(not(any(unix, target_os = "windows")))]
    let _ = path;
    Ok(())
}

pub(crate) fn harden_private_file(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(target_os = "windows")]
    {
        validate_windows_path(path, false)?;
        apply_windows_private_acl(path, false)?;
    }
    #[cfg(not(any(unix, target_os = "windows")))]
    let _ = path;
    Ok(())
}

#[cfg(target_os = "windows")]
fn validate_windows_path(path: &Path, directory: bool) -> io::Result<()> {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || if directory {
            !metadata.is_dir()
        } else {
            !metadata.is_file()
        }
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "private application paths must not be links or reparse points",
        ));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn apply_windows_private_acl(path: &Path, directory: bool) -> io::Result<()> {
    use std::{os::windows::ffi::OsStrExt, ptr};
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::{
            Authorization::{
                ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
            },
            DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
            SetFileSecurityW,
        },
    };

    // Name the current token user explicitly. OWNER RIGHTS would be unsafe for
    // a restored object owned by another account (or by the Administrators
    // group), because replacing its DACL could lock out the process that is
    // currently repairing it. SYSTEM and local Administrators retain recovery
    // access. Inheritance is enabled only for the data directory.
    let current_user = windows_current_user_sid()?;
    let sddl = if directory {
        format!("D:P(A;OICI;FA;;;{current_user})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)")
    } else {
        format!("D:P(A;;FA;;;{current_user})(A;;FA;;;SY)(A;;FA;;;BA)")
    };
    let sddl = std::ffi::OsStr::new(&sddl)
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let wide_path = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: both UTF-16 buffers are NUL-terminated and live for the calls.
    // The conversion API initializes `descriptor` with LocalAlloc storage,
    // which is released exactly once below regardless of SetFileSecurityW's
    // result.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let applied = unsafe {
        SetFileSecurityW(
            wide_path.as_ptr(),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            descriptor,
        )
    };
    let error = (applied == 0).then(io::Error::last_os_error);
    unsafe {
        let _ = LocalFree(descriptor.cast());
    }
    match error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

#[cfg(target_os = "windows")]
fn windows_current_user_sid() -> io::Result<String> {
    use std::{mem::size_of, ptr, slice};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, LocalFree},
        Security::{
            Authorization::ConvertSidToStringSidW, GetTokenInformation, TOKEN_QUERY, TOKEN_USER,
            TokenUser,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };

    let mut token = ptr::null_mut();
    // SAFETY: GetCurrentProcess returns a valid pseudo-handle, and `token`
    // points to writable storage for the resulting real handle.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }

    let result = (|| {
        let mut required = 0_u32;
        // The first call intentionally supplies no buffer so Windows reports
        // the exact TOKEN_USER storage size through `required`.
        unsafe {
            GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut required);
        }
        if required < size_of::<TOKEN_USER>() as u32 {
            return Err(io::Error::last_os_error());
        }

        // A usize vector supplies alignment suitable for TOKEN_USER while the
        // API continues to receive and report its length in bytes.
        let words = (required as usize).div_ceil(size_of::<usize>());
        let mut storage = vec![0_usize; words];
        if unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                storage.as_mut_ptr().cast(),
                required,
                &mut required,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let token_user = unsafe { &*storage.as_ptr().cast::<TOKEN_USER>() };
        let mut string_sid = ptr::null_mut();
        if unsafe { ConvertSidToStringSidW(token_user.User.Sid, &mut string_sid) } == 0 {
            return Err(io::Error::last_os_error());
        }

        let decoded = (|| {
            let mut length = 0_usize;
            // Text SID strings are short ASCII strings. Bound the scan anyway
            // so a malformed platform result cannot walk arbitrary memory.
            while length <= 256 && unsafe { *string_sid.add(length) } != 0 {
                length += 1;
            }
            if length > 256 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Windows returned an invalid current-user SID",
                ));
            }
            let value = String::from_utf16(unsafe { slice::from_raw_parts(string_sid, length) })
                .map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Windows returned an invalid current-user SID",
                    )
                })?;
            let Some(components) = value.strip_prefix("S-1-") else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Windows returned an invalid current-user SID",
                ));
            };
            if components.is_empty()
                || !components
                    .chars()
                    .all(|character| character.is_ascii_digit() || character == '-')
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Windows returned an invalid current-user SID",
                ));
            }
            Ok(value)
        })();
        unsafe {
            let _ = LocalFree(string_sid.cast());
        }
        decoded
    })();

    unsafe {
        let _ = CloseHandle(token);
    }
    result
}
