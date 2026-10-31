//! Per-user process exclusion independent of the selected Codex config directory.

use directories::BaseDirs;
use fs2::FileExt;
use std::{
    error::Error,
    fmt,
    fs::{self, File, OpenOptions},
    io,
    path::Path,
};

#[cfg(target_os = "linux")]
use gpui_kit::{AppContext as _, ParentElement as _, Styled as _};

const APP_DIRECTORY: &str = "Codex Auth Switcher";
const LOCK_FILE: &str = "instance.lock";

pub(crate) struct InstanceLock {
    // Keep the handle alive for the app lifetime; the OS releases its lock when it closes.
    _file: File,
}

#[derive(Debug)]
pub(crate) enum AcquireError {
    AlreadyRunning,
    UserDataDirectoryUnavailable,
    Io {
        operation: &'static str,
        source: io::Error,
    },
}

impl fmt::Display for AcquireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyRunning => f.write_str("程序已在运行。"),
            Self::UserDataDirectoryUnavailable => f.write_str("无法定位当前用户的数据目录。"),
            Self::Io { operation, source } => write!(f, "{operation}：{source}"),
        }
    }
}

impl Error for AcquireError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::AlreadyRunning | Self::UserDataDirectoryUnavailable => None,
        }
    }
}

pub(crate) fn acquire() -> Result<InstanceLock, AcquireError> {
    let base_dirs = BaseDirs::new().ok_or(AcquireError::UserDataDirectoryUnavailable)?;
    acquire_at(
        &base_dirs
            .data_local_dir()
            .join(APP_DIRECTORY)
            .join(LOCK_FILE),
    )
}

fn acquire_at(path: &Path) -> Result<InstanceLock, AcquireError> {
    let directory = path.parent().ok_or_else(|| AcquireError::Io {
        operation: "单实例锁目录路径无效",
        source: io::Error::new(io::ErrorKind::InvalidInput, "missing parent directory"),
    })?;
    create_private_directory(directory).map_err(|source| AcquireError::Io {
        operation: "无法创建或保护单实例锁目录",
        source,
    })?;

    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path).map_err(|source| AcquireError::Io {
        operation: "无法打开单实例锁文件",
        source,
    })?;
    match file.try_lock_exclusive() {
        Ok(()) => Ok(InstanceLock { _file: file }),
        Err(error) if is_lock_occupied(&error) => Err(AcquireError::AlreadyRunning),
        Err(source) => Err(AcquireError::Io {
            operation: "无法取得单实例锁",
            source,
        }),
    }
}

fn is_lock_occupied(error: &io::Error) -> bool {
    if error.kind() == io::ErrorKind::WouldBlock {
        return true;
    }
    #[cfg(windows)]
    {
        error.raw_os_error() == Some(windows_sys::Win32::Foundation::ERROR_LOCK_VIOLATION as i32)
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn create_private_directory(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "lock directory is not a regular directory",
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir_all(path)?,
        Err(error) => return Err(error),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub(crate) fn show_message(title: &str, description: &str, level: rfd::MessageLevel) {
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    rfd::MessageDialog::new()
        .set_title(title)
        .set_description(description)
        .set_level(level)
        .show();

    #[cfg(target_os = "linux")]
    show_gpui_message(title, description, level);
}

#[cfg(target_os = "linux")]
struct NoticeWindow {
    heading: String,
    description: String,
}

#[cfg(target_os = "linux")]
impl gpui_kit::Render for NoticeWindow {
    fn render(
        &mut self,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        gpui_kit::div()
            .flex()
            .flex_col()
            .gap_2()
            .p_5()
            .child(self.heading.clone())
            .child(self.description.clone())
    }
}

#[cfg(target_os = "linux")]
fn show_gpui_message(title: &str, description: &str, level: rfd::MessageLevel) {
    let severity = match level {
        rfd::MessageLevel::Info => "提示",
        rfd::MessageLevel::Warning => "警告",
        rfd::MessageLevel::Error => "错误",
    };
    let heading = format!("{severity}：{title}");
    let description = description.to_owned();
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let options = gpui_kit::WindowOptions {
            window_bounds: Some(gpui_kit::WindowBounds::Windowed(
                gpui_kit::Bounds::centered(
                    None,
                    gpui_kit::size(gpui_kit::px(460.), gpui_kit::px(170.)),
                    cx,
                ),
            )),
            titlebar: Some(gpui_kit::TitlebarOptions {
                title: Some("Codex Auth Switcher".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        if gpui_kit::open_window(options, cx, |_, cx| {
            cx.new(|_| NoticeWindow {
                heading,
                description,
            })
        })
        .is_err()
        {
            eprintln!("Cannot open the GPUI single-instance notice window.");
            cx.quit();
        }
        cx.activate(true);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        process::{Child, Command, Stdio},
        thread,
        time::{Duration, Instant},
    };

    const TEST_LOCK_PATH: &str = "CODEX_AUTH_SWITCHER_TEST_LOCK_PATH";
    const TEST_READY_PATH: &str = "CODEX_AUTH_SWITCHER_TEST_READY_PATH";
    const TEST_RELEASE_PATH: &str = "CODEX_AUTH_SWITCHER_TEST_RELEASE_PATH";

    #[test]
    fn child_process_lock_holder() -> Result<(), Box<dyn Error>> {
        let Some(lock_path) = std::env::var_os(TEST_LOCK_PATH) else {
            return Ok(());
        };
        let ready_path = std::env::var_os(TEST_READY_PATH)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing ready path"))?;
        let release_path = std::env::var_os(TEST_RELEASE_PATH)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing release path"))?;
        let _lock = acquire_at(Path::new(&lock_path))?;
        fs::write(ready_path, b"locked")?;

        let deadline = Instant::now() + Duration::from_secs(10);
        while !Path::new(&release_path).exists() {
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "parent did not release child lock test",
                )
                .into());
            }
            thread::sleep(Duration::from_millis(10));
        }

        // Exit without dropping the guard to verify the operating system releases the lock.
        std::process::exit(0);
    }

    #[test]
    fn lock_is_exclusive_across_processes_and_released_when_process_exits()
    -> Result<(), Box<dyn Error>> {
        let temporary = tempfile::tempdir()?;
        let lock_path = temporary.path().join(LOCK_FILE);
        let ready_path = temporary.path().join("ready");
        let release_path = temporary.path().join("release");
        let mut child = Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "single_instance::tests::child_process_lock_holder",
                "--nocapture",
            ])
            .env(TEST_LOCK_PATH, &lock_path)
            .env(TEST_READY_PATH, &ready_path)
            .env(TEST_RELEASE_PATH, &release_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;

        let ready = wait_for_file(&mut child, &ready_path);
        let contention = match acquire_at(&lock_path) {
            Err(AcquireError::AlreadyRunning) => Ok(()),
            Ok(lock) => {
                drop(lock);
                Err("second process acquired an occupied lock".to_owned())
            }
            Err(error) => Err(format!(
                "lock attempt returned an unexpected error: {error:?}"
            )),
        };
        fs::write(&release_path, b"exit")?;
        let child_status = child.wait()?;

        assert!(ready.is_ok(), "child process did not acquire the lock");
        if let Err(error) = contention {
            return Err(io::Error::other(error).into());
        }
        assert!(child_status.success(), "child process did not exit cleanly");
        let _reacquired = acquire_at(&lock_path)?;
        Ok(())
    }

    fn wait_for_file(child: &mut Child, path: &Path) -> io::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if path.is_file() {
                return Ok(());
            }
            if let Some(status) = child.try_wait()? {
                return Err(io::Error::other(format!(
                    "child process exited before acquiring the lock: {status}"
                )));
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "child process did not acquire the lock in time",
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}
