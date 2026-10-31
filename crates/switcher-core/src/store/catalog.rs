use super::{Active, Journal, Store};
use crate::{
    Error, Result, config,
    crypto::Cipher,
    files::{self, Lock},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
use toml_edit::{DocumentMut, value};

/// Editable view of the Codex model catalog, bound to the file version it loaded.
#[derive(Clone)]
pub struct ModelCatalog {
    pub path: String,
    pub models: Vec<CodexModel>,
    original: Vec<u8>,
    extra: BTreeMap<String, serde_json::Value>,
    home: PathBuf,
}

/// Known fields that users can safely manage while preserving all other model metadata.
#[derive(Clone, Serialize, Deserialize)]
pub struct CodexModel {
    pub slug: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub visibility: String,
    #[serde(default)]
    pub default_reasoning_level: String,
    #[serde(default)]
    pub supported_reasoning_levels: Vec<ReasoningLevel>,
    #[serde(default)]
    pub priority: i64,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ReasoningLevel {
    pub effort: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_json::Value>,
}

impl ReasoningLevel {
    pub fn new(effort: String, description: String) -> Self {
        Self {
            effort,
            description,
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct ModelCatalogDocument {
    models: Vec<CodexModel>,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Serialize)]
pub struct ModelCatalogSaveResult {
    pub backup: Option<String>,
    pub model_count: usize,
}

/// Resolve the path explicitly selected by Codex in `config.toml`.
///
/// A missing setting means that Codex owns the catalog selection internally;
/// the switcher must not guess a filename such as `models-codex.json`.
/// Relative paths are resolved from the directory containing `config.toml`,
/// matching Codex's configuration loader.
pub(super) fn configured_model_catalog_path(
    doc: &DocumentMut,
    config_dir: &Path,
) -> Result<Option<PathBuf>> {
    let Some(item) = doc.get("model_catalog_json") else {
        return Ok(None);
    };
    let path = item
        .as_str()
        .ok_or(Error("Model catalog path must be a string."))?;
    if path.is_empty() || path.len() > 4096 || path.chars().any(|character| character.is_control())
    {
        return Err(Error("Model catalog path is invalid."));
    }
    let path = PathBuf::from(path);
    Ok(Some(if path.is_absolute() {
        path
    } else {
        config_dir.join(path)
    }))
}

fn configured_catalog_path(store: &Store) -> Result<PathBuf> {
    let bytes = files::read(&store.paths.config(), files::CONFIG_LIMIT)?.ok_or(Error(
        "Codex config.toml was not found. Choose a valid directory.",
    ))?;
    let document = config::parse(&bytes)?;
    configured_model_catalog_path(&document, &store.paths.home)?.ok_or(Error(
        "Codex model catalog path is not configured in config.toml.",
    ))
}

fn selected_catalog_path(path: &Path, config_dir: &Path) -> Result<(PathBuf, String)> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        config_dir.join(path)
    };
    let text = path
        .to_str()
        .ok_or(Error("Model catalog path must be valid Unicode."))?;
    if text.is_empty() || text.len() > 4096 || text.chars().any(char::is_control) {
        return Err(Error("Model catalog path is invalid."));
    }
    let text = text.to_string();
    Ok((path, text))
}

fn read_model_catalog(path: &Path, path_text: String, home: &Path) -> Result<ModelCatalog> {
    let original = files::read(path, files::CONFIG_LIMIT)?
        .ok_or(Error("Selected Codex model catalog was not found."))?;
    let document: ModelCatalogDocument = serde_json::from_slice(&original)
        .map_err(|_| Error("Selected Codex model catalog JSON is invalid."))?;
    validate_model_catalog(&document.models)?;
    Ok(ModelCatalog {
        path: path_text,
        models: document.models,
        original,
        extra: document.extra,
        home: home.to_path_buf(),
    })
}

fn config_with_model_catalog_path(config_bytes: &[u8], path: &str) -> Result<Vec<u8>> {
    let mut document = config::parse(config_bytes)?;
    document["model_catalog_json"] = value(path);
    serialize_config(document)
}

fn serialize_config(document: DocumentMut) -> Result<Vec<u8>> {
    let mut output = document
        .to_string()
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    if !output.ends_with('\n') {
        output.push('\n');
    }
    config::parse(output.as_bytes())?;
    Ok(output.into_bytes())
}

impl Store {
    /// Load the Codex model catalog and retain its source bytes for conflict detection.
    pub fn load_model_catalog(&self) -> Result<ModelCatalog> {
        let path = configured_catalog_path(self)?;
        let path_text = path.to_string_lossy().into_owned();
        read_model_catalog(&path, path_text, &self.paths.home)
    }

    /// Configure an existing catalog for the active mode, then return its loaded model list.
    ///
    /// The selected file is validated before any write. If a mode vault exists, the active
    /// profile receives only the catalog-path change so imported model settings remain intact.
    pub fn configure_model_catalog(&self, path: &Path) -> Result<ModelCatalog> {
        let _lock = Lock::acquire(&self.paths)?;
        self.legacy_pending()?;
        if self.paths.journal().exists() {
            return Err(Error(
                "Recovery is pending. Initialize modes to recover before selecting a model catalog.",
            ));
        }

        let before_config = files::read(&self.paths.config(), files::CONFIG_LIMIT)?.ok_or(
            Error("Codex config.toml was not found. Initialize the Codex configuration first."),
        )?;
        let (selected_path, selected_text) = selected_catalog_path(path, &self.paths.home)?;
        let catalog = read_model_catalog(&selected_path, selected_text.clone(), &self.paths.home)?;
        let after_config = config_with_model_catalog_path(&before_config, &selected_text)?;
        let before = Active {
            config: before_config,
            auth: files::read(&self.paths.auth(), files::AUTH_LIMIT)?,
        };
        let after = Active {
            config: after_config,
            auth: before.auth.clone(),
        };
        let config_changed = before.config != after.config;

        let mut cipher = None;
        let mut vault_before = None;
        let mut vault_after = None;
        let mut vault_changed = false;
        if files::read(&self.paths.vault(), files::STATE_LIMIT)?.is_some() {
            let opened = Cipher::open(&self.paths, false)?;
            let (mut state, raw_vault) = self.state(&opened)?;
            let mode = Self::kind(&before.config, before.auth.as_deref())?.ok_or(Error(
                "Cannot determine the active mode. Select a model catalog after activating a mode.",
            ))?;
            if before.auth.is_some() && config::auth_mode(before.auth.as_deref())? != Some(mode) {
                return Err(Error(
                    "The active configuration and authentication do not match one mode.",
                ));
            }
            if let Some(profile) = state.profiles.get_mut(&mode) {
                let updated =
                    config_with_model_catalog_path(profile.config.as_bytes(), &selected_text)?;
                let updated = String::from_utf8(updated)
                    .map_err(|_| Error("Cannot encode the saved mode configuration."))?;
                if profile.config != updated {
                    profile.config = updated;
                    vault_changed = true;
                }
            }
            vault_before = Some(raw_vault.clone());
            vault_after = Some(if vault_changed {
                Self::encode(&opened, "modes", &state)?
            } else {
                raw_vault
            });
            cipher = Some(opened);
        }

        if !config_changed && !vault_changed {
            return Ok(catalog);
        }
        if self.active()? != before
            || files::read(&self.paths.vault(), files::STATE_LIMIT)? != vault_before
        {
            return Err(Error(
                "Files changed while configuring the model catalog. Refresh and retry.",
            ));
        }
        if config_changed {
            let backup = super::profile_backup(&before.config)?;
            self.backup(backup.as_bytes())?;
        }

        if !vault_changed {
            files::write(&self.paths.config(), &after.config)?;
            return Ok(catalog);
        }

        let cipher = cipher.ok_or(Error("Cannot open the encrypted mode archive."))?;
        let vault_before = vault_before.ok_or(Error("The encrypted mode archive is missing."))?;
        let vault_after = vault_after.ok_or(Error("The updated mode archive is missing."))?;
        let record = Journal {
            version: 2,
            before,
            after,
            vault_before,
            vault_after,
        };
        files::write(
            &self.paths.journal(),
            &Self::encode(&cipher, "pending", &record)?,
        )?;
        let operation = (|| {
            if self.active()? != record.before
                || files::read(&self.paths.vault(), files::STATE_LIMIT)?.as_deref()
                    != Some(record.vault_before.as_slice())
            {
                return Err(Error("Files changed before configuring the model catalog."));
            }
            if config_changed {
                files::write(&self.paths.config(), &record.after.config)?;
            }
            #[cfg(test)]
            if self.fail_catalog_after_config {
                return Err(Error("Simulated model catalog configuration failure."));
            }
            files::write(&self.paths.vault(), &record.vault_after)?;
            if self.active()? != record.after
                || files::read(&self.paths.vault(), files::STATE_LIMIT)?.as_deref()
                    != Some(record.vault_after.as_slice())
            {
                return Err(Error("External changes interrupted model catalog setup."));
            }
            std::fs::remove_file(self.paths.journal())
                .map_err(|_| Error("Cannot clear completed recovery record."))
        })();
        if operation.is_err() {
            return match self.recover(&cipher) {
                Ok(()) => Err(Error(
                    "Model catalog setup failed; the previous configuration and mode archive were restored.",
                )),
                Err(_) => Err(Error(
                    "Model catalog setup is incomplete. Preserve pending.enc and initialize to recover.",
                )),
            };
        }
        Ok(catalog)
    }

    /// Back up and atomically save validated options without overwriting concurrent edits.
    pub fn save_model_catalog(
        &self,
        mut catalog: ModelCatalog,
    ) -> Result<(ModelCatalog, ModelCatalogSaveResult)> {
        if catalog.home != self.paths.home {
            return Err(Error(
                "The model catalog belongs to another Codex directory.",
            ));
        }
        validate_model_catalog(&catalog.models)?;
        let _lock = Lock::acquire(&self.paths)?;
        let path = configured_catalog_path(self)?;
        if catalog.path != path.to_string_lossy() {
            return Err(Error(
                "Codex model catalog path changed after it was loaded. Reload before saving.",
            ));
        }
        let current = files::read(&path, files::CONFIG_LIMIT)?
            .ok_or(Error("Configured Codex model catalog was not found."))?;
        if current != catalog.original {
            return Err(Error(
                "The model catalog changed after it was loaded. Reload before saving.",
            ));
        }
        let document = ModelCatalogDocument {
            models: catalog.models.clone(),
            extra: catalog.extra.clone(),
        };
        let mut updated = serde_json::to_vec_pretty(&document)
            .map_err(|_| Error("Cannot serialize the Codex model catalog."))?;
        updated.push(b'\n');
        let backup = if updated == current {
            None
        } else {
            let backup = self.backup_model_catalog(&path, &current)?;
            files::write(&path, &updated)?;
            catalog.original.clone_from(&updated);
            Some(backup.to_string_lossy().into_owned())
        };
        Ok((
            catalog,
            ModelCatalogSaveResult {
                backup,
                model_count: document.models.len(),
            },
        ))
    }

    fn backup_model_catalog(&self, path: &Path, bytes: &[u8]) -> Result<PathBuf> {
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let parent = path.parent().unwrap_or(&self.paths.home);
        let file_name = path
            .file_name()
            .ok_or(Error("Invalid model catalog path."))?
            .to_string_lossy();
        for i in 0..1000 {
            let suffix = if i == 0 {
                String::new()
            } else {
                format!("-{i:02}")
            };
            let backup = parent.join(format!("{file_name}.bak-{stamp}{suffix}"));
            if backup.exists() {
                continue;
            }
            files::create(&backup, bytes)?;
            return Ok(backup);
        }
        Err(Error("Cannot reserve a unique model catalog backup."))
    }
}

fn validate_model_catalog(models: &[CodexModel]) -> Result<()> {
    if models.is_empty() || models.len() > 512 {
        return Err(Error(
            "Codex model catalog must contain between 1 and 512 models.",
        ));
    }
    let mut slugs = BTreeSet::new();
    for model in models {
        let slug_valid = !model.slug.is_empty()
            && model.slug.len() <= 128
            && model
                .slug
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte));
        if !slug_valid || !slugs.insert(model.slug.as_str()) {
            return Err(Error("Model IDs must be valid and unique."));
        }
        if model.display_name.trim().is_empty()
            || model.display_name.chars().count() > 120
            || model.display_name.chars().any(char::is_control)
        {
            return Err(Error(
                "Model display names must be non-empty and at most 120 characters.",
            ));
        }
        if model.visibility.trim().is_empty()
            || model.visibility.len() > 32
            || model.visibility.chars().any(char::is_control)
        {
            return Err(Error("Model visibility must be a short non-empty value."));
        }
        if model.supported_reasoning_levels.is_empty()
            || model.supported_reasoning_levels.len() > 32
        {
            return Err(Error(
                "Each model must have between 1 and 32 reasoning levels.",
            ));
        }
        let mut efforts = BTreeSet::new();
        for level in &model.supported_reasoning_levels {
            if level.effort.is_empty()
                || level.effort.len() > 32
                || !level
                    .effort
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
                || level.description.chars().count() > 240
                || level.description.chars().any(char::is_control)
                || !efforts.insert(level.effort.as_str())
            {
                return Err(Error("Model reasoning levels must be valid and unique."));
            }
        }
        if model.default_reasoning_level.is_empty()
            || !efforts.contains(model.default_reasoning_level.as_str())
        {
            return Err(Error(
                "The default reasoning level must be supported by its model.",
            ));
        }
    }
    Ok(())
}
