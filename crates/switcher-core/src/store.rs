//! Coordinated config/auth updates with an encrypted rollback record.

use crate::{
    Error, Mode, Paths, Result, config,
    crypto::Cipher,
    files::{self, Lock},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use toml_edit::{DocumentMut, Item, Table, value};
use zeroize::Zeroizing;

#[derive(Clone, Default, Serialize, Deserialize)]
struct Profile {
    config: String,
    auth: Option<String>,
    config_source: String,
    auth_source: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct State {
    version: u8,
    profiles: BTreeMap<Mode, Profile>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            version: 2,
            profiles: BTreeMap::new(),
        }
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
struct Active {
    config: Vec<u8>,
    auth: Option<Vec<u8>>,
}

#[derive(Serialize, Deserialize)]
struct Journal {
    version: u8,
    before: Active,
    after: Active,
    vault_before: Vec<u8>,
    vault_after: Vec<u8>,
}

#[derive(Clone, Serialize)]
pub struct ModeStatus {
    pub mode: Mode,
    pub provider: String,
    pub model: String,
    pub effort: String,
    pub base_url: String,
    pub ready: bool,
    pub issue: String,
    pub config_source: String,
    pub auth_source: String,
}

#[derive(Clone, Serialize)]
pub struct Status {
    pub config_path: String,
    pub mode: Option<Mode>,
    pub auth_mode: Option<Mode>,
    pub provider: String,
    pub model: String,
    pub effort: String,
    pub base_url: String,
    pub initialized: bool,
    pub recovery_pending: bool,
    pub modes: Vec<ModeStatus>,
}

#[derive(Clone, Serialize)]
pub struct Change {
    pub field: String,
    pub before: String,
    pub after: String,
}

#[derive(Clone, Serialize)]
pub struct Preview {
    pub target: Mode,
    pub changes: Vec<Change>,
    pub auth_changes: bool,
    pub auth_source: String,
    pub changed: bool,
    pub warning: String,
}

#[derive(Default)]
pub struct SwitchOptions {
    pub provider: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<Zeroizing<String>>,
}

#[derive(Default)]
pub struct ImportSources {
    pub subscription_config: Option<PathBuf>,
    pub subscription_auth: Option<PathBuf>,
    pub api_config: Option<PathBuf>,
    pub api_auth: Option<PathBuf>,
}

/// Opaque preparation binds previewed values to the exact config, auth and vault.
pub struct PreparedSwitch {
    home: PathBuf,
    before: Active,
    after: Active,
    vault_before: Vec<u8>,
    state: State,
    persist_api_key: Option<Zeroizing<String>>,
    pub preview: Preview,
}

#[derive(Serialize)]
pub struct SwitchResult {
    pub target: Mode,
    pub changed: bool,
    pub backup: Option<String>,
    pub restart_required: bool,
    pub environment_persisted: Option<bool>,
}

#[derive(Clone)]
pub struct Store {
    pub paths: Paths,
    inherit_environment: bool,
    #[cfg(test)]
    fail_after_auth: bool,
    #[cfg(test)]
    environment: BTreeMap<String, String>,
}

impl Store {
    pub fn new(paths: Paths) -> Self {
        Self {
            paths,
            inherit_environment: true,
            #[cfg(test)]
            fail_after_auth: false,
            #[cfg(test)]
            environment: BTreeMap::new(),
        }
    }

    /// Demo and tests must never read credentials from the host environment.
    pub fn isolated(paths: Paths) -> Self {
        Self {
            inherit_environment: false,
            ..Self::new(paths)
        }
    }

    fn environment(&self, name: &str) -> Result<Option<Zeroizing<String>>> {
        #[cfg(test)]
        if let Some(value) = self.environment.get(name) {
            return Ok(Some(Zeroizing::new(value.clone())));
        }
        if self.inherit_environment {
            environment_key(&self.paths, name)
        } else {
            Ok(None)
        }
    }

    fn active(&self) -> Result<Active> {
        Ok(Active {
            config: files::read(&self.paths.config(), files::CONFIG_LIMIT)?.ok_or(Error(
                "Codex config.toml was not found. Choose a valid directory.",
            ))?,
            auth: files::read(&self.paths.auth(), files::AUTH_LIMIT)?,
        })
    }

    fn file_auth(document: &DocumentMut) -> Result<()> {
        if document
            .get("cli_auth_credentials_store")
            .and_then(Item::as_str)
            .is_some_and(|s| s != "file")
        {
            return Err(Error("Only Codex file authentication is supported."));
        }
        Ok(())
    }

    fn kind(config: &[u8], auth: Option<&[u8]>) -> Result<Option<Mode>> {
        let doc = config::parse(config)?;
        if config::table(&doc)?.is_some_and(|t| t.get("env_key").is_some()) {
            return Ok(Some(Mode::Api));
        }
        config::auth_mode(auth)
    }

    fn state(&self, cipher: &Cipher) -> Result<(State, Vec<u8>)> {
        let raw = files::read(&self.paths.vault(), files::STATE_LIMIT)?
            .ok_or(Error("Initialize modes before switching."))?;
        let plain = cipher.decrypt("modes", &raw)?;
        let state: State =
            serde_json::from_slice(&plain).map_err(|_| Error("Invalid encrypted mode data."))?;
        if state.version != 2 || state.profiles.len() > 2 {
            return Err(Error("Unsupported mode state version."));
        }
        Ok((state, raw))
    }

    fn encode<T: Serialize>(cipher: &Cipher, purpose: &str, data: &T) -> Result<Vec<u8>> {
        let plain = Zeroizing::new(
            serde_json::to_vec(data).map_err(|_| Error("Cannot serialize local state."))?,
        );
        cipher.encrypt(purpose, &plain)
    }

    fn legacy_pending(&self) -> Result<()> {
        if self.paths.home.join("auth-switcher/pending.dpapi").exists() {
            return Err(Error(
                "Version one has a pending write. Recover it with version one before continuing.",
            ));
        }
        Ok(())
    }

    fn recover(&self, cipher: &Cipher) -> Result<()> {
        let Some(raw) = files::read(&self.paths.journal(), files::STATE_LIMIT)? else {
            return Ok(());
        };
        let plain = cipher.decrypt("pending", &raw)?;
        let record: Journal = serde_json::from_slice(&plain)
            .map_err(|_| Error("Invalid encrypted recovery record."))?;
        if record.version != 2 {
            return Err(Error("Unsupported recovery record version."));
        }
        let active = self.active()?;
        let vault = files::read(&self.paths.vault(), files::STATE_LIMIT)?
            .ok_or(Error("Mode vault is missing during recovery."))?;
        if (active.config != record.before.config && active.config != record.after.config)
            || (active.auth != record.before.auth && active.auth != record.after.auth)
            || (vault != record.vault_before && vault != record.vault_after)
        {
            return Err(Error(
                "External changes block recovery. Preserve pending.enc and resolve the conflict.",
            ));
        }
        if active.config != record.before.config {
            files::write(&self.paths.config(), &record.before.config)?;
        }
        if active.auth != record.before.auth {
            self.restore_auth(record.before.auth.as_deref())?;
        }
        if vault != record.vault_before {
            files::write(&self.paths.vault(), &record.vault_before)?;
        }
        fs::remove_file(self.paths.journal())
            .map_err(|_| Error("Cannot remove completed recovery record."))
    }

    fn restore_auth(&self, bytes: Option<&[u8]>) -> Result<()> {
        match bytes {
            Some(bytes) => files::write(&self.paths.auth(), bytes),
            None => {
                if self.paths.auth().exists() {
                    fs::remove_file(self.paths.auth())
                        .map_err(|_| Error("Cannot restore missing authentication state."))?;
                }
                Ok(())
            }
        }
    }

    pub fn status(&self) -> Result<Status> {
        let active = self.active()?;
        let doc = config::parse(&active.config)?;
        let state = if self.paths.vault().exists() {
            self.state(&Cipher::open(&self.paths, false)?)?.0
        } else {
            State::default()
        };
        let mut modes = Vec::new();
        for (mode, profile) in state.profiles {
            let snapshot = config::parse(profile.config.as_bytes())?;
            let auth = decode_auth(profile.auth.as_deref())?;
            let issue = config::validate_profile(mode, &profile.config, auth.as_deref())
                .and_then(|()| self.check_model(&snapshot))
                .err()
                .map(|e| e.to_string())
                .unwrap_or_default();
            modes.push(ModeStatus {
                mode,
                provider: config::provider(&snapshot)?.into(),
                model: config::summary(&snapshot, "model")?,
                effort: config::summary(&snapshot, "model_reasoning_effort")?,
                base_url: base_url(&snapshot)?,
                ready: issue.is_empty(),
                issue,
                config_source: profile.config_source,
                auth_source: profile.auth_source,
            });
        }
        Ok(Status {
            config_path: self.paths.config().to_string_lossy().into(),
            mode: Self::kind(&active.config, active.auth.as_deref())?,
            auth_mode: config::auth_mode(active.auth.as_deref())?,
            provider: config::provider(&doc)?.into(),
            model: config::summary(&doc, "model")?,
            effort: config::summary(&doc, "model_reasoning_effort")?,
            base_url: base_url(&doc)?,
            initialized: self.paths.vault().exists(),
            recovery_pending: self.paths.journal().exists(),
            modes,
        })
    }

    pub fn initialize(&self, sources: ImportSources) -> Result<Status> {
        let _lock = Lock::acquire(&self.paths)?;
        self.legacy_pending()?;
        if self.paths.journal().exists() {
            self.recover(&Cipher::open(&self.paths, false)?)?;
        }
        let active = self.active()?;
        let doc = config::parse(&active.config)?;
        Self::file_auth(&doc)?;
        let mut state = if self.paths.vault().exists() {
            self.state(&Cipher::open(&self.paths, false)?)?.0
        } else {
            State::default()
        };
        for mode in [Mode::Subscription, Mode::Api] {
            let (source_config, source_auth) = match mode {
                Mode::Subscription => (&sources.subscription_config, &sources.subscription_auth),
                Mode::Api => (&sources.api_config, &sources.api_auth),
            };
            let profile = state.profiles.entry(mode).or_default();
            if source_config.is_some() || profile.config.is_empty() {
                let candidates = match source_config {
                    Some(path) => vec![path.clone()],
                    None => self.candidates(&self.paths.config(), "config.toml.bak-")?,
                };
                for path in candidates {
                    let imported = (|| {
                        let bytes = files::read(&path, files::CONFIG_LIMIT)?
                            .ok_or(Error("Configuration source is missing."))?;
                        let snapshot = config::parse(&bytes)?;
                        Self::file_auth(&snapshot)?;
                        let inferred = if path == self.paths.config() {
                            Self::kind(&bytes, active.auth.as_deref())?
                        } else {
                            Some(if config::provider(&snapshot)? == "openai" {
                                Mode::Subscription
                            } else {
                                Mode::Api
                            })
                        };
                        if source_config.is_none() && inferred != Some(mode) {
                            return Ok(None);
                        }
                        let fragment = config::fragment(&snapshot)?;
                        if config::has_credentials(config::parse(fragment.as_bytes())?.as_item()) {
                            return Err(Error("Imported provider contains credentials in TOML."));
                        }
                        Ok(Some(fragment))
                    })();
                    match imported {
                        Ok(Some(fragment)) => {
                            profile.config = fragment;
                            profile.config_source = file_name(&path);
                            break;
                        }
                        Ok(None) => {}
                        Err(error) if source_config.is_some() || path == self.paths.config() => {
                            return Err(error);
                        }
                        Err(_) => {}
                    }
                }
            }
            if mode == Mode::Api && profile.config.is_empty() {
                profile.config = api_template(
                    &doc,
                    self.environment("AUTOCODE_BASE_URL")?
                        .as_deref()
                        .map(String::as_str),
                )?;
                profile.config_source = "default API parameters".into();
            }
            if source_auth.is_some() || profile.auth.is_none() {
                let candidates = match source_auth {
                    Some(path) => vec![path.clone()],
                    None => self.candidates(&self.paths.auth(), "auth.json.bak")?,
                };
                for path in candidates {
                    let imported = (|| {
                        let bytes = files::read(&path, files::AUTH_LIMIT)?;
                        if config::auth_mode(bytes.as_deref())? != Some(mode) {
                            if source_auth.is_some() {
                                return Err(Error(
                                    "Authentication source belongs to another mode.",
                                ));
                            }
                            return Ok(None);
                        }
                        Ok(bytes)
                    })();
                    match imported {
                        Ok(Some(auth)) => {
                            profile.auth = Some(STANDARD.encode(auth));
                            profile.auth_source = file_name(&path);
                            break;
                        }
                        Ok(None) => {}
                        Err(error) if source_auth.is_some() => return Err(error),
                        Err(_) => {}
                    }
                }
            }
        }
        if self.active()? != active {
            return Err(Error("Codex files changed during initialization. Retry."));
        }
        let cipher = Cipher::open(&self.paths, true)?;
        files::write(
            &self.paths.vault(),
            &Self::encode(&cipher, "modes", &state)?,
        )?;
        self.status()
    }

    fn candidates(&self, current: &Path, prefix: &str) -> Result<Vec<PathBuf>> {
        let mut backups = Vec::new();
        for entry in
            fs::read_dir(&self.paths.home).map_err(|_| Error("Cannot list Codex backups."))?
        {
            let entry = entry.map_err(|_| Error("Cannot inspect backup entries."))?;
            if entry.file_name().to_string_lossy().starts_with(prefix) {
                backups.push(entry.path());
            }
            if backups.len() > 256 {
                return Err(Error("Too many backups. Select explicit import files."));
            }
        }
        backups.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
        backups.insert(0, current.to_path_buf());
        Ok(backups)
    }

    pub fn prepare(&self, target: Mode, options: SwitchOptions) -> Result<PreparedSwitch> {
        self.legacy_pending()?;
        if self.paths.journal().exists() {
            return Err(Error(
                "Recovery is pending. Initialize modes to recover before previewing.",
            ));
        }
        let before = self.active()?;
        let current = config::parse(&before.config)?;
        Self::file_auth(&current)?;
        let leaving = Self::kind(&before.config, before.auth.as_deref())?.ok_or(Error(
            "Current authentication is missing. Import valid authentication first.",
        ))?;
        if config::auth_mode(before.auth.as_deref())? != Some(leaving) {
            return Err(Error("Current provider and authentication do not match."));
        }
        let cipher = Cipher::open(&self.paths, false)?;
        let (mut state, vault_before) = self.state(&cipher)?;
        let current_fragment = config::fragment(&current)?;
        let current_auth = before.auth.as_ref().map(|auth| STANDARD.encode(auth));
        let preserve_imported_profile = state.profiles.get(&leaving).is_some_and(|profile| {
            profile.config_source != "latest active config"
                && (profile.config != current_fragment || profile.auth != current_auth)
        });
        if !preserve_imported_profile {
            state.profiles.insert(
                leaving,
                Profile {
                    config: current_fragment,
                    auth: current_auth,
                    config_source: "latest active config".into(),
                    auth_source: "latest active authentication".into(),
                },
            );
        }
        let destination = state.profiles.get_mut(&target).ok_or(Error(
            "Target mode configuration is missing. Initialize or import its config.",
        ))?;
        let mut auth_source = destination.auth_source.clone();
        let mut fragment = config::parse(destination.config.as_bytes())?;
        let api_key_was_supplied = options.api_key.is_some();
        let mut persist_api_key = None;
        let provider_env = self.environment(if target == Mode::Api {
            "CODEX_API_PROVIDER"
        } else {
            "CODEX_MODEL_PROVIDER"
        })?;
        if let Some(name) = options
            .provider
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .or(provider_env.as_deref().map(String::as_str))
        {
            config::validate_provider(name)?;
            if name != config::provider(&fragment)? {
                let selected = config::table(&fragment)?.cloned();
                fragment["model_provider"] = value(name);
                fragment.remove("model_providers");
                if let Some(selected) = selected {
                    let mut providers = Table::new();
                    providers.insert(name, Item::Table(selected));
                    fragment["model_providers"] = Item::Table(providers);
                }
            }
        }
        if target == Mode::Api {
            if let Some(key) = options.api_key.filter(|key| !key.is_empty()) {
                destination.auth = Some(STANDARD.encode(config::api_auth(&key)?));
                auth_source = "user input".into();
                persist_api_key = Some(key);
            }
            let name = config::provider(&fragment)?.to_string();
            let table = fragment
                .get_mut("model_providers")
                .and_then(Item::as_table_mut)
                .and_then(|t| t.get_mut(&name))
                .and_then(Item::as_table_mut)
                .ok_or(Error("Target API provider table is missing."))?;
            if table.contains_key("env_key") {
                if destination.auth.is_none() {
                    return Err(Error("API key is missing. Enter a key before switching."));
                }
                table.remove("env_key");
                table.insert("requires_openai_auth", value(true));
            }
            let env_base = self.environment("AUTOCODE_BASE_URL")?;
            if let Some(url) = options
                .base_url
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .or(env_base.as_deref().map(String::as_str))
            {
                config::endpoint(url)?;
                table.insert("base_url", value(url));
            }
        } else if options.base_url.is_some() || api_key_was_supplied {
            return Err(Error("API input requires API mode."));
        }
        destination.config = fragment.to_string();
        // Once a profile is applied, the active files become its authoritative snapshot.
        destination.config_source = "latest active config".into();
        destination.auth_source = "latest active authentication".into();
        let auth = decode_auth(destination.auth.as_deref())?;
        config::validate_profile(target, &destination.config, auth.as_deref())?;
        self.check_model(&fragment)?;
        let after_config = if target == leaving
            && config::parse(config::fragment(&current)?.as_bytes())?.to_string()
                == fragment.to_string()
        {
            before.config.clone()
        } else {
            config::apply(&before.config, &destination.config)?
        };
        if after_config != before.config && config::has_credentials(current.as_item()) {
            return Err(Error(
                "Current config contains credentials. Move them out before creating a config backup.",
            ));
        }
        let after = Active {
            config: after_config,
            auth,
        };
        let after_doc = config::parse(&after.config)?;
        let mut changes = Vec::new();
        for field in ["model_provider", "model", "model_reasoning_effort"] {
            let old = config::summary(&current, field)?;
            let new = config::summary(&after_doc, field)?;
            if old != new {
                changes.push(Change {
                    field: field.into(),
                    before: old,
                    after: new,
                });
            }
        }
        let old = base_url(&current)?;
        let new = base_url(&after_doc)?;
        if old != new {
            changes.push(Change {
                field: "base_url".into(),
                before: old,
                after: new,
            });
        }
        for field in config::MODE_FIELDS {
            if ["model_provider", "model", "model_reasoning_effort"].contains(field) {
                continue;
            }
            if current.get(field).map(ToString::to_string)
                != after_doc.get(field).map(ToString::to_string)
            {
                changes.push(Change {
                    field: (*field).into(),
                    before: if current.get(field).is_some() {
                        "configured"
                    } else {
                        "not set"
                    }
                    .into(),
                    after: if after_doc.get(field).is_some() {
                        "configured"
                    } else {
                        "not set"
                    }
                    .into(),
                });
            }
        }
        let preview = Preview { target, changed: before != after, auth_changes: before.auth != after.auth, auth_source, changes, warning: "Close Codex before applying. Restart Codex and use a new chat after switching; old chats may retain their provider.".into() };
        Ok(PreparedSwitch {
            home: self.paths.home.clone(),
            before,
            after,
            vault_before,
            state,
            persist_api_key,
            preview,
        })
    }

    pub fn apply(&self, prepared: PreparedSwitch) -> Result<SwitchResult> {
        let _lock = Lock::acquire(&self.paths)?;
        self.legacy_pending()?;
        if prepared.home != self.paths.home || self.paths.journal().exists() {
            return Err(Error(
                "The preview belongs to a different directory or recovery is pending.",
            ));
        }
        if self.active()? != prepared.before
            || files::read(&self.paths.vault(), files::STATE_LIMIT)?.as_deref()
                != Some(prepared.vault_before.as_slice())
        {
            return Err(Error(
                "Files changed since preview. Refresh and preview again.",
            ));
        }
        let cipher = Cipher::open(&self.paths, false)?;
        self.state(&cipher)?;
        let backup = if prepared.before.config != prepared.after.config {
            Some(self.backup(&prepared.before.config)?)
        } else {
            None
        };
        let vault_after = Self::encode(&cipher, "modes", &prepared.state)?;
        let record = Journal {
            version: 2,
            before: prepared.before,
            after: prepared.after,
            vault_before: prepared.vault_before,
            vault_after,
        };
        files::write(
            &self.paths.journal(),
            &Self::encode(&cipher, "pending", &record)?,
        )?;
        let operation = (|| {
            if self.active()? != record.before {
                return Err(Error("Files changed before applying."));
            }
            if record.before.auth != record.after.auth {
                self.restore_auth(record.after.auth.as_deref())?;
            }
            #[cfg(test)]
            if self.fail_after_auth {
                return Err(Error("Simulated file write failure."));
            }
            let intermediate = self.active()?;
            if intermediate.auth != record.after.auth || intermediate.config != record.before.config
            {
                return Err(Error("External changes interrupted the switch."));
            }
            if record.before.config != record.after.config {
                files::write(&self.paths.config(), &record.after.config)?;
            }
            files::write(&self.paths.vault(), &record.vault_after)?;
            if self.active()? != record.after {
                return Err(Error("External changes interrupted verification."));
            }
            fs::remove_file(self.paths.journal())
                .map_err(|_| Error("Cannot clear completed recovery record."))?;
            Ok(())
        })();
        if operation.is_err() {
            return match self.recover(&cipher) {
                Ok(()) => Err(Error(
                    "Switch failed; previous configuration and authentication were restored.",
                )),
                Err(_) => Err(Error(
                    "Switch incomplete. Preserve pending.enc, close Codex and initialize to recover.",
                )),
            };
        }
        let environment_persisted = prepared
            .persist_api_key
            .as_deref()
            .map(|key| crate::environment::persist_api_key(key));
        Ok(SwitchResult {
            target: prepared.preview.target,
            changed: prepared.preview.changed,
            backup: backup.map(|p| p.to_string_lossy().into()),
            restart_required: prepared.preview.changed,
            environment_persisted,
        })
    }

    fn backup(&self, bytes: &[u8]) -> Result<PathBuf> {
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        for i in 0..1000 {
            let suffix = if i == 0 {
                String::new()
            } else {
                format!("-{i:02}")
            };
            let path = self
                .paths
                .home
                .join(format!("config.toml.bak-{stamp}{suffix}"));
            if path.exists() {
                continue;
            }
            files::create(&path, bytes)?;
            return Ok(path);
        }
        Err(Error("Cannot reserve a unique configuration backup."))
    }

    fn check_model(&self, doc: &DocumentMut) -> Result<()> {
        let Some(model) = doc.get("model").and_then(Item::as_str) else {
            return Ok(());
        };
        let explicit = doc.get("model_catalog_json").and_then(Item::as_str);
        let candidates = explicit
            .map(|s| vec![self.paths.home.join(s)])
            .unwrap_or_else(|| {
                ["models_cache.json", "models-codex.json", "models.json"]
                    .iter()
                    .map(|s| self.paths.home.join(s))
                    .collect()
            });
        for path in candidates {
            let Some(bytes) = files::read(&path, files::CONFIG_LIMIT)? else {
                if explicit.is_some() {
                    return Err(Error("Target model catalog is missing."));
                }
                continue;
            };
            let json: serde_json::Value = serde_json::from_slice(&bytes)
                .map_err(|_| Error("Invalid local model catalog."))?;
            let models = json
                .get("models")
                .or(Some(&json))
                .and_then(serde_json::Value::as_array)
                .ok_or(Error("Invalid local model list."))?;
            if let Some(entry) = models.iter().find(|e| {
                e.get("slug")
                    .or_else(|| e.get("id"))
                    .and_then(serde_json::Value::as_str)
                    == Some(model)
            }) {
                if let (Some(effort), Some(levels)) = (
                    doc.get("model_reasoning_effort").and_then(Item::as_str),
                    entry
                        .get("supported_reasoning_levels")
                        .and_then(serde_json::Value::as_array),
                ) && !levels.is_empty()
                    && !levels
                        .iter()
                        .any(|v| v.get("effort").unwrap_or(v).as_str() == Some(effort))
                {
                    return Err(Error(
                        "Reasoning effort is not supported by the selected model.",
                    ));
                }
                return Ok(());
            }
            if explicit.is_some() {
                return Err(Error("Target model is absent from its catalog."));
            }
        }
        Ok(())
    }
}

fn decode_auth(value: Option<&str>) -> Result<Option<Vec<u8>>> {
    value
        .map(|s| {
            STANDARD
                .decode(s)
                .map_err(|_| Error("Invalid saved authentication encoding."))
        })
        .transpose()
}
fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "selected file".into())
}
fn base_url(doc: &DocumentMut) -> Result<String> {
    let url = config::table(doc)?
        .and_then(|t| t.get("base_url"))
        .and_then(Item::as_str)
        .unwrap_or("");
    if !url.is_empty() {
        config::endpoint(url)?;
    }
    Ok(url.into())
}

fn api_template(current: &DocumentMut, endpoint: Option<&str>) -> Result<String> {
    let mut doc = config::parse(config::fragment(current)?.as_bytes())?;
    doc["model_provider"] = value("OpenAI");
    doc.remove("model_providers");
    let url = endpoint.unwrap_or("https://vip.auto-code.net/v1");
    config::endpoint(url)?;
    doc["model_providers"] = Item::Table(Table::new());
    let mut table = Table::new();
    for (key, val) in [
        ("name", value("API key")),
        ("base_url", value(url)),
        ("wire_api", value("responses")),
        ("requires_openai_auth", value(true)),
    ] {
        table.insert(key, val);
    }
    doc["model_providers"]["OpenAI"] = Item::Table(table);
    if doc.contains_key("forced_login_method") {
        doc["forced_login_method"] = value("api");
    }
    doc.remove("forced_chatgpt_workspace_id");
    Ok(doc.to_string())
}

fn environment_key(paths: &Paths, name: &str) -> Result<Option<Zeroizing<String>>> {
    if let Ok(key) = std::env::var(name) {
        let key = Zeroizing::new(key);
        if !key.trim().is_empty() {
            return Ok(Some(Zeroizing::new(key.trim().into())));
        }
    }
    #[cfg(windows)]
    {
        use winreg::{
            RegKey,
            enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE},
        };
        for (hive, subkey) in [
            (HKEY_CURRENT_USER, "Environment"),
            (
                HKEY_LOCAL_MACHINE,
                r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
            ),
        ] {
            if let Ok(registry) = RegKey::predef(hive).open_subkey(subkey)
                && let Ok(key) = registry.get_value::<String, _>(name)
            {
                let key = Zeroizing::new(key);
                if !key.trim().is_empty() {
                    return Ok(Some(Zeroizing::new(key.trim().into())));
                }
            }
        }
    }
    if let Some(bytes) = files::read(&paths.home.join(".env"), files::AUTH_LIMIT)? {
        let bytes = Zeroizing::new(bytes);
        let text = std::str::from_utf8(&bytes).map_err(|_| Error("Codex .env must be UTF-8."))?;
        for line in text.lines() {
            if let Some((key, val)) = line.trim().trim_start_matches("export ").split_once('=')
                && key.trim() == name
            {
                let val = val.trim();
                let val = if val.len() >= 2
                    && ((val.starts_with('\"') && val.ends_with('\"'))
                        || (val.starts_with('\'') && val.ends_with('\'')))
                {
                    &val[1..val.len() - 1]
                } else {
                    val
                };
                return Ok((!val.is_empty()).then(|| Zeroizing::new(val.into())));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests;
