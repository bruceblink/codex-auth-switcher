#[cfg(not(windows))]
use switcher_core::Error;
use switcher_core::Result;

pub fn supported() -> bool {
    cfg!(windows)
}

pub fn restart() -> Result<()> {
    #[cfg(windows)]
    {
        windows::restart()
    }
    #[cfg(not(windows))]
    {
        Err(Error(
            "Restarting the Codex desktop app is only supported on Windows.",
        ))
    }
}

#[cfg(windows)]
mod windows {
    use std::{
        collections::{BTreeMap, BTreeSet},
        ffi::{OsStr, OsString},
        mem::size_of,
        os::windows::{
            ffi::{OsStrExt, OsStringExt},
            process::CommandExt,
        },
        path::{Component, Path, PathBuf},
        process::{Command, Stdio},
        ptr, thread,
        time::{Duration, Instant},
    };

    use switcher_core::{Error, Result};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE, HWND, INVALID_HANDLE_VALUE, LPARAM},
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
        System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
            QueryFullProcessImageNameW, WaitForSingleObject,
        },
        UI::{
            Shell::ShellExecuteW,
            WindowsAndMessaging::{
                EnumWindows, GW_OWNER, GetWindow, GetWindowThreadProcessId, IsWindowVisible,
                PostMessageW, SW_SHOWNORMAL, WM_CLOSE,
            },
        },
    };

    const POWERSHELL_TIMEOUT: Duration = Duration::from_secs(8);
    const CLOSE_TIMEOUT: Duration = Duration::from_secs(15);
    const START_TIMEOUT: Duration = Duration::from_secs(30);
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const WAIT_OBJECT_0: u32 = 0;
    const WAIT_TIMEOUT: u32 = 258;

    struct CodexApp {
        aumid: String,
        executable: PathBuf,
    }

    struct ProcessHandle(HANDLE);

    impl Drop for ProcessHandle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    struct AppWindows {
        windows: Vec<HWND>,
        processes: BTreeMap<u32, ProcessHandle>,
    }

    struct WindowSearch {
        executable: String,
        windows: Vec<HWND>,
        processes: BTreeMap<u32, ProcessHandle>,
    }

    pub(super) fn restart() -> Result<()> {
        let app = installed_app()?;
        let old_windows = app_windows(&app.executable)?;
        let running_processes = running_app_processes(&app.executable)?;
        let old_pids: BTreeSet<u32> = old_windows.processes.keys().copied().collect();
        if running_processes
            .iter()
            .any(|process_id| !old_pids.contains(process_id))
        {
            return Err(Error(
                "Codex is running without an open desktop window. Exit it from the system tray, then retry.",
            ));
        }
        close_and_wait(old_windows)?;
        activate(&app.aumid)?;
        wait_for_window(&app.executable, &old_pids)
    }

    fn installed_app() -> Result<CodexApp> {
        let script = r#"
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$package = Get-AppxPackage -Name 'OpenAI.Codex' -ErrorAction SilentlyContinue | Select-Object -First 1
if ($null -eq $package) { exit 10 }
$application = (Get-AppxPackageManifest -Package $package).Package.Applications.Application |
    Where-Object { $_.Executable -match '(^|[/\\])ChatGPT\.exe$' -and $_.EntryPoint -eq 'Windows.FullTrustApplication' } |
    Select-Object -First 1
if ($null -eq $application) { exit 11 }
[Console]::Out.WriteLine((@($package.PackageFamilyName, $application.Id, $package.InstallLocation, $application.Executable) -join "`t"))
"#;
        let output = run_hidden_powershell(script)?;
        if !output.status.success() {
            return Err(Error(
                "The OpenAI Codex desktop app is not installed or could not be inspected.",
            ));
        }
        let stdout = String::from_utf8(output.stdout)
            .map_err(|_| Error("The OpenAI Codex desktop package information is invalid."))?;
        let (family_name, app_id, install_location, executable) = parse_package_details(&stdout)?;
        let executable = PathBuf::from(executable);
        if !Path::new(&install_location).is_absolute()
            || executable.is_absolute()
            || executable
                .components()
                .any(|component| matches!(component, Component::ParentDir))
        {
            return Err(Error(
                "The OpenAI Codex desktop package information is invalid.",
            ));
        }
        let executable = PathBuf::from(install_location).join(executable);
        let aumid = format!("{family_name}!{app_id}");
        Ok(CodexApp { aumid, executable })
    }

    fn parse_package_details(output: &str) -> Result<(&str, &str, &str, &str)> {
        let mut fields = output.trim().split('\t');
        let family_name = fields.next().unwrap_or_default();
        let app_id = fields.next().unwrap_or_default();
        let install_location = fields.next().unwrap_or_default();
        let executable = fields.next().unwrap_or_default();
        if fields.next().is_some()
            || family_name.is_empty()
            || app_id.is_empty()
            || install_location.is_empty()
            || executable.is_empty()
            || !family_name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
            || !app_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
            || family_name.len() + app_id.len() + 1 > 256
        {
            return Err(Error(
                "The OpenAI Codex desktop package information is invalid.",
            ));
        }
        Ok((family_name, app_id, install_location, executable))
    }

    fn run_hidden_powershell(script: &str) -> Result<std::process::Output> {
        let system_root =
            std::env::var_os("SystemRoot").ok_or(Error("Could not locate Windows PowerShell."))?;
        let powershell = PathBuf::from(system_root)
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe");
        let mut child = Command::new(powershell)
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-WindowStyle",
                "Hidden",
                "-Command",
                script,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|_| Error("Could not inspect the OpenAI Codex desktop installation."))?;
        let deadline = Instant::now() + POWERSHELL_TIMEOUT;
        loop {
            if child
                .try_wait()
                .map_err(|_| Error("Could not inspect the OpenAI Codex desktop installation."))?
                .is_some()
            {
                break;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error(
                    "Timed out inspecting the OpenAI Codex desktop installation.",
                ));
            }
            thread::sleep(Duration::from_millis(50));
        }
        child
            .wait_with_output()
            .map_err(|_| Error("Could not inspect the OpenAI Codex desktop installation."))
    }

    fn app_windows(executable: &Path) -> Result<AppWindows> {
        let mut search = WindowSearch {
            executable: executable.to_string_lossy().into_owned(),
            windows: Vec::new(),
            processes: BTreeMap::new(),
        };
        let context = &mut search as *mut WindowSearch as LPARAM;
        if unsafe { EnumWindows(Some(collect_app_windows), context) } == 0 {
            return Err(Error("Could not inspect Codex desktop windows."));
        }
        Ok(AppWindows {
            windows: search.windows,
            processes: search.processes,
        })
    }

    fn running_app_processes(executable: &Path) -> Result<BTreeSet<u32>> {
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(Error("Could not inspect running Codex desktop processes."));
        }
        let _snapshot = ProcessHandle(snapshot);
        let expected_name = executable.file_name().and_then(OsStr::to_str).ok_or(Error(
            "The OpenAI Codex desktop package information is invalid.",
        ))?;
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if unsafe { Process32FirstW(snapshot, &mut entry) } == 0 {
            return Err(Error("Could not inspect running Codex desktop processes."));
        }

        let mut process_ids = BTreeSet::new();
        loop {
            let name_length = entry
                .szExeFile
                .iter()
                .position(|character| *character == 0)
                .unwrap_or(entry.szExeFile.len());
            let process_name = OsString::from_wide(&entry.szExeFile[..name_length]);
            if process_name
                .to_string_lossy()
                .eq_ignore_ascii_case(expected_name)
            {
                let process_id = entry.th32ProcessID;
                let handle =
                    unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
                if !handle.is_null() {
                    let _process = ProcessHandle(handle);
                    if process_image_path(handle).is_some_and(|path| {
                        path.eq_ignore_ascii_case(&executable.to_string_lossy())
                    }) {
                        process_ids.insert(process_id);
                    }
                }
            }
            if unsafe { Process32NextW(snapshot, &mut entry) } == 0 {
                break;
            }
        }
        Ok(process_ids)
    }

    unsafe extern "system" fn collect_app_windows(hwnd: HWND, context: LPARAM) -> i32 {
        if unsafe { IsWindowVisible(hwnd) } == 0 || !unsafe { GetWindow(hwnd, GW_OWNER) }.is_null()
        {
            return 1;
        }
        let mut process_id = 0u32;
        if unsafe { GetWindowThreadProcessId(hwnd, &mut process_id) } == 0 {
            return 1;
        }
        let access = PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION;
        let handle = unsafe { OpenProcess(access, 0, process_id) };
        if handle.is_null() {
            return 1;
        }
        let process = ProcessHandle(handle);
        let Some(image_path) = process_image_path(handle) else {
            return 1;
        };
        // The app is a packaged ChatGPT.exe; codex.exe CLI and other packages do not match.
        let search = unsafe { &mut *(context as *mut WindowSearch) };
        if !image_path.eq_ignore_ascii_case(&search.executable) {
            return 1;
        }
        search.windows.push(hwnd);
        if let std::collections::btree_map::Entry::Vacant(entry) =
            search.processes.entry(process_id)
        {
            entry.insert(process);
        }
        1
    }

    fn process_image_path(handle: HANDLE) -> Option<String> {
        let mut buffer = vec![0u16; 32768];
        let mut length = buffer.len() as u32;
        if unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut length) } == 0 {
            return None;
        }
        Some(
            OsString::from_wide(&buffer[..length as usize])
                .to_string_lossy()
                .into_owned(),
        )
    }

    fn close_and_wait(app: AppWindows) -> Result<()> {
        if app.windows.is_empty() {
            return Ok(());
        }
        for window in app.windows {
            if unsafe { PostMessageW(window, WM_CLOSE, 0, 0) } == 0 {
                return Err(Error(
                    "Could not ask Codex to close normally. Close it manually and retry.",
                ));
            }
        }
        let deadline = Instant::now() + CLOSE_TIMEOUT;
        for process in app.processes.values() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() || wait_process(process.0, remaining)? == WAIT_TIMEOUT {
                return Err(Error(
                    "Codex did not exit within 15 seconds. Save your work, close it manually, and retry.",
                ));
            }
        }
        Ok(())
    }

    fn wait_process(handle: HANDLE, timeout: Duration) -> Result<u32> {
        let milliseconds = timeout.as_millis().min(u32::MAX as u128 - 1) as u32;
        let status = unsafe { WaitForSingleObject(handle, milliseconds) };
        if status == WAIT_OBJECT_0 || status == WAIT_TIMEOUT {
            return Ok(status);
        }
        Err(Error("Could not confirm that Codex closed normally."))
    }

    fn activate(aumid: &str) -> Result<()> {
        let shell_path = wide(&format!("shell:AppsFolder\\{aumid}"));
        let verb = wide("open");
        let result = unsafe {
            ShellExecuteW(
                ptr::null_mut(),
                verb.as_ptr(),
                shell_path.as_ptr(),
                ptr::null(),
                ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        if result as isize <= 32 {
            return Err(Error("Windows could not launch the Codex desktop app."));
        }
        Ok(())
    }

    fn wait_for_window(executable: &Path, old_process_ids: &BTreeSet<u32>) -> Result<()> {
        let deadline = Instant::now() + START_TIMEOUT;
        loop {
            if app_windows(executable)?
                .processes
                .keys()
                .any(|process_id| !old_process_ids.contains(process_id))
            {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(Error(
                    "Codex did not open within 30 seconds. Open it manually and retry.",
                ));
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    fn wide(value: &str) -> Vec<u16> {
        OsStr::new(value).encode_wide().chain(Some(0)).collect()
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use windows_sys::Win32::UI::WindowsAndMessaging::{MB_OK, MessageBoxW};

        #[test]
        fn installed_package_discovery_for_local_verification() -> Result<()> {
            if std::env::var_os("CODEX_AUTH_SWITCHER_VERIFY_INSTALLED_APP").is_some() {
                let app = installed_app()?;
                assert!(app.executable.is_file());
            }
            Ok(())
        }

        #[test]
        fn package_details_build_version_independent_app_identity() -> Result<()> {
            let (family, app_id, install, executable) = parse_package_details(
                "OpenAI.Codex_2p2nqsd0c76g0\tApp\tC:\\Program Files\\WindowsApps\\OpenAI.Codex_1.0.0_x64__2p2nqsd0c76g0\tapp/ChatGPT.exe\r\n",
            )?;
            let install = PathBuf::from(install);
            let executable = install.join(executable);
            assert_eq!(
                format!("{family}!{app_id}"),
                "OpenAI.Codex_2p2nqsd0c76g0!App"
            );
            assert_eq!(
                executable,
                PathBuf::from(
                    r"C:\Program Files\WindowsApps\OpenAI.Codex_1.0.0_x64__2p2nqsd0c76g0\app\ChatGPT.exe"
                )
            );
            Ok(())
        }

        #[test]
        fn package_details_reject_incomplete_or_unsafe_identity() {
            assert!(
                parse_package_details("OpenAI.Codex\tApp\tC:\\app\tapp/ChatGPT.exe	extra").is_err()
            );
            assert!(
                parse_package_details("OpenAI.Codex\tApp!Other\tC:\\app\tapp/ChatGPT.exe").is_err()
            );
        }

        #[test]
        fn normal_close_waits_for_a_controlled_native_window_process() -> Result<()> {
            const CHILD_ENV: &str = "CODEX_AUTH_SWITCHER_RESTART_TEST_WINDOW";
            if std::env::var_os(CHILD_ENV).is_some() {
                let message = wide("Controlled Codex restart test window");
                let title = wide("Test window");
                unsafe {
                    MessageBoxW(ptr::null_mut(), message.as_ptr(), title.as_ptr(), MB_OK);
                }
                return Ok(());
            }

            let executable = std::env::current_exe()
                .map_err(|_| Error("Could not start the controlled window test."))?;
            let mut child = Command::new(&executable)
                .args([
                    "--exact",
                    "restart::windows::tests::normal_close_waits_for_a_controlled_native_window_process",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(CHILD_ENV, "1")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
                .map_err(|_| Error("Could not start the controlled window test."))?;
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut child_windows = None;
            while Instant::now() < deadline {
                let candidate = match app_windows(&executable) {
                    Ok(candidate) => candidate,
                    Err(error) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(error);
                    }
                };
                if candidate.processes.contains_key(&child.id()) {
                    child_windows = Some(candidate);
                    break;
                }
                if child
                    .try_wait()
                    .map_err(|_| Error("Could not inspect the controlled window test."))?
                    .is_some()
                {
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }
            let Some(child_windows) = child_windows else {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error("The controlled native window did not appear."));
            };

            if let Err(error) = close_and_wait(child_windows) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
            let status = child
                .wait()
                .map_err(|_| Error("Could not confirm the controlled window test exited."))?;
            assert!(status.success());
            Ok(())
        }
    }
}
