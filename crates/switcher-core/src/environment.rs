#[cfg(test)]
pub(crate) fn persist_api_key(key: &str) -> bool {
    // Tests use synthetic keys and must never write to the current user's environment.
    !key.is_empty()
}

#[cfg(windows)]
fn write_user_environment_value(environment: &winreg::RegKey, name: &str, value: &str) -> bool {
    environment.set_value(name, &value).is_ok()
}

#[cfg(all(windows, not(test)))]
pub(crate) fn persist_api_key(key: &str) -> bool {
    use std::{ffi::OsStr, os::windows::ffi::OsStrExt};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
    };
    use winreg::{
        RegKey,
        enums::{HKEY_CURRENT_USER, KEY_SET_VALUE},
    };

    let user = RegKey::predef(HKEY_CURRENT_USER);
    let environment = user
        .open_subkey_with_flags("Environment", KEY_SET_VALUE)
        .or_else(|_| user.create_subkey("Environment").map(|(key, _)| key));
    let Ok(environment) = environment else {
        return false;
    };
    if !write_user_environment_value(&environment, "AUTOCODE_API_KEY", key) {
        return false;
    }

    // Notify Windows apps that read the user environment at launch; the registry write is
    // already durable even if a receiver ignores this bounded broadcast.
    let setting = OsStr::new("Environment")
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut ignored_result = 0usize;
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            setting.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            5_000,
            &mut ignored_result,
        );
    }
    true
}

#[cfg(all(not(windows), not(test)))]
pub(crate) fn persist_api_key(_key: &str) -> bool {
    false
}

#[cfg(all(test, windows))]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};
    use winreg::{RegKey, enums::HKEY_CURRENT_USER};

    struct Cleanup(String);

    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(&self.0);
        }
    }

    #[test]
    fn registry_writer_uses_only_the_supplied_test_key() -> anyhow::Result<()> {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = format!("Software\\CodexAuthSwitcherTests\\{nonce:x}");
        let root = RegKey::predef(HKEY_CURRENT_USER);
        let (test_key, _) = root.create_subkey(&path)?;
        let _cleanup = Cleanup(path);
        assert!(super::write_user_environment_value(
            &test_key,
            "AUTOCODE_API_KEY",
            "synthetic-registry-value"
        ));
        assert_eq!(
            test_key.get_value::<String, _>("AUTOCODE_API_KEY")?,
            "synthetic-registry-value"
        );
        Ok(())
    }
}
