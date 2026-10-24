//! Bounded reads, exclusive creation and same-directory replacements.

use crate::{Error, Paths, Result};
use fs2::FileExt;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

pub const CONFIG_LIMIT: usize = 4 * 1024 * 1024;
pub const AUTH_LIMIT: usize = 256 * 1024;
pub const STATE_LIMIT: usize = 24 * 1024 * 1024;

pub fn read(path: &Path, limit: usize) -> Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Ok(meta) if !meta.is_file() || meta.file_type().is_symlink() => {
            return Err(Error(
                "Expected a regular file; symbolic links are not supported.",
            ));
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(Error("Cannot inspect a local file. Check permissions.")),
    }
    let mut data = Vec::new();
    File::open(path)
        .map_err(|_| Error("Cannot open a local file."))?
        .take(limit as u64 + 1)
        .read_to_end(&mut data)
        .map_err(|_| Error("Cannot read a local file."))?;
    if data.len() > limit {
        return Err(Error("File exceeds the supported size limit."));
    }
    Ok(Some(data))
}

pub fn write(path: &Path, data: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or(Error("Invalid destination directory."))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|_| Error("Cannot create a temporary file."))?;
    temporary
        .write_all(data)
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|_| Error("Cannot write a local file."))?;
    temporary
        .persist(path)
        .map_err(|_| Error("Cannot replace a local file. Close other programs and retry."))?;
    Ok(())
}

pub fn create(path: &Path, data: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|_| Error("Cannot create a private file without overwriting existing data."))?;
    file.write_all(data)
        .and_then(|()| file.sync_all())
        .map_err(|_| Error("Cannot save a private file."))
}

pub fn private_directory(path: &Path) -> Result<()> {
    if let Ok(meta) = fs::symlink_metadata(path)
        && (!meta.is_dir() || meta.file_type().is_symlink())
    {
        return Err(Error("State directory must be a regular directory."));
    }
    fs::create_dir_all(path).map_err(|_| Error("Cannot create the user state directory."))?;
    #[cfg(windows)]
    crate::permissions::restrict_directory(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| Error("Cannot restrict state directory permissions."))?;
    }
    Ok(())
}

pub struct Lock(File);
impl Lock {
    pub fn acquire(paths: &Paths) -> Result<Self> {
        private_directory(&paths.state())?;
        let path = paths.state().join("operation.lock");
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(path)
            .map_err(|_| Error("Cannot open operation lock."))?;
        file.try_lock_exclusive()
            .map_err(|_| Error("Another switcher operation is running."))?;
        Ok(Self(file))
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}
