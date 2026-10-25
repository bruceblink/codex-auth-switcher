//! Local configuration and encrypted mode storage. Errors never contain input values.

pub mod config;
pub mod crypto;
mod environment;
mod files;
#[cfg(windows)]
mod permissions;
mod store;

pub use store::{
    CodexModel, ImportSources, ModeStatus, ModelCatalog, ModelCatalogSaveResult, PreparedSwitch,
    Preview, ReasoningLevel, Status, Store, SwitchOptions, SwitchResult,
};

use serde::{Deserialize, Serialize};
use std::{fmt, path::PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Subscription,
    Api,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Subscription => "subscription",
            Self::Api => "api",
        }
    }
}

#[derive(Debug)]
pub struct Error(pub &'static str);
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone)]
pub struct Paths {
    pub home: PathBuf,
}
impl Paths {
    pub fn resolve(home: Option<PathBuf>) -> Result<Self> {
        let home = home
            .or_else(|| {
                std::env::var_os("CODEX_HOME")
                    .filter(|v| !v.is_empty())
                    .map(PathBuf::from)
            })
            .or_else(|| directories::BaseDirs::new().map(|d| d.home_dir().join(".codex")))
            .ok_or(Error("Cannot locate the Codex user directory."))?;
        let home = if home.is_absolute() {
            home
        } else {
            std::env::current_dir()
                .map_err(|_| Error("Cannot resolve the Codex directory."))?
                .join(home)
        };
        Ok(Self { home })
    }
    pub fn config(&self) -> PathBuf {
        self.home.join("config.toml")
    }
    pub fn auth(&self) -> PathBuf {
        self.home.join("auth.json")
    }
    pub fn models_catalog(&self) -> PathBuf {
        self.home.join("models-codex.json")
    }
    pub fn state(&self) -> PathBuf {
        self.home.join("auth-switcher-v2")
    }
    pub fn vault(&self) -> PathBuf {
        self.state().join("modes.enc")
    }
    pub fn journal(&self) -> PathBuf {
        self.state().join("pending.enc")
    }
    pub fn key(&self) -> PathBuf {
        self.state().join("vault.key")
    }
}
