use semver::Version;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdatePlatform {
    WindowsX64,
    LinuxAmd64Deb,
    LinuxX86_64AppImage,
    MacosArm64,
    MacosX86_64,
}

impl UpdatePlatform {
    pub(super) fn asset_name(self, version: &Version) -> String {
        match self {
            Self::WindowsX64 => {
                format!("Codex-Auth-Switcher-{version}-windows-x64-setup.exe")
            }
            Self::LinuxAmd64Deb => {
                format!("Codex-Auth-Switcher-{version}-linux-amd64.deb")
            }
            Self::LinuxX86_64AppImage => {
                format!("Codex-Auth-Switcher-{version}-linux-x86_64.AppImage")
            }
            Self::MacosArm64 => {
                format!("Codex-Auth-Switcher-{version}-macos-arm64.dmg")
            }
            Self::MacosX86_64 => {
                format!("Codex-Auth-Switcher-{version}-macos-x86_64.dmg")
            }
        }
    }
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
pub fn current_platform() -> Option<UpdatePlatform> {
    Some(UpdatePlatform::WindowsX64)
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub fn current_platform() -> Option<UpdatePlatform> {
    let appimage = std::env::var_os("APPIMAGE").is_some_and(|value| !value.is_empty());
    if appimage {
        Some(UpdatePlatform::LinuxX86_64AppImage)
    } else {
        Some(UpdatePlatform::LinuxAmd64Deb)
    }
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub fn current_platform() -> Option<UpdatePlatform> {
    Some(UpdatePlatform::MacosArm64)
}

#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
pub fn current_platform() -> Option<UpdatePlatform> {
    Some(UpdatePlatform::MacosX86_64)
}

#[cfg(not(any(
    all(target_os = "windows", target_arch = "x86_64"),
    all(target_os = "linux", target_arch = "x86_64"),
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "macos", target_arch = "x86_64")
)))]
pub fn current_platform() -> Option<UpdatePlatform> {
    None
}
