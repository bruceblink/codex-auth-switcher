use super::Store;
use crate::{
    Error, Result, config,
    files::{self, Lock},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
use toml_edit::DocumentMut;

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

impl Store {
    /// Load the Codex model catalog and retain its source bytes for conflict detection.
    pub fn load_model_catalog(&self) -> Result<ModelCatalog> {
        let path = configured_catalog_path(self)?;
        let original = files::read(&path, files::CONFIG_LIMIT)?
            .ok_or(Error("Configured Codex model catalog was not found."))?;
        let document: ModelCatalogDocument = serde_json::from_slice(&original)
            .map_err(|_| Error("Codex model catalog JSON is invalid."))?;
        validate_model_catalog(&document.models)?;
        Ok(ModelCatalog {
            path: path.to_string_lossy().into_owned(),
            models: document.models,
            original,
            extra: document.extra,
            home: self.paths.home.clone(),
        })
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
