#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
//! A standalone GPUI Kit desktop application. No remote requests.
mod app;
#[cfg(test)]
mod demo;
mod restart;
mod single_instance;

fn main() -> anyhow::Result<()> {
    let instance_lock = match single_instance::acquire() {
        Ok(lock) => lock,
        Err(single_instance::AcquireError::AlreadyRunning) => {
            single_instance::show_message(
                "程序已在运行",
                "Codex Auth Switcher 已经打开。请使用现有窗口。",
                rfd::MessageLevel::Info,
            );
            return Ok(());
        }
        Err(error) => {
            single_instance::show_message(
                "无法启动 Codex Auth Switcher",
                &format!("无法取得单实例锁。请检查当前用户数据目录的权限。\n{error}"),
                rfd::MessageLevel::Error,
            );
            return Err(anyhow::Error::new(error));
        }
    };

    let paths = switcher_core::Paths::resolve(None)?;
    let result = app::run(switcher_core::Store::new(paths));
    drop(instance_lock);
    result
}
