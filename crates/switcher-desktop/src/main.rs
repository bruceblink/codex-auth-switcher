#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
//! A standalone egui desktop application. No remote requests.
mod app;
#[cfg(test)]
mod demo;
mod restart;

fn main() -> anyhow::Result<()> {
    let paths = switcher_core::Paths::resolve(None)?;
    app::run(switcher_core::Store::new(paths))
}
