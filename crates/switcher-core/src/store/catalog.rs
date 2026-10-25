use super::Store;
use crate::{
    Error, Result,
    files::{self, Lock},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

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

impl Store {
    /// Load the Codex model catalog and retain its source bytes for conflict detection.
    pub fn load_model_catalog(&self) -> Result<ModelCatalog> {
        let path = self.paths.models_catalog();
        let original = files::read(&path, files::CONFIG_LIMIT)?
            .ok_or(Error("Codex models-codex.json was not found."))?;
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
        if catalog.home != self.paths.home
            || catalog.path != self.paths.models_catalog().to_string_lossy()
        {
            return Err(Error(
                "The model catalog belongs to another Codex directory.",
            ));
        }
        validate_model_catalog(&catalog.models)?;
        let _lock = Lock::acquire(&self.paths)?;
        let path = self.paths.models_catalog();
        let current = files::read(&path, files::CONFIG_LIMIT)?
            .ok_or(Error("Codex models-codex.json was not found."))?;
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
            let backup = self.backup_model_catalog(&current)?;
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

    fn backup_model_catalog(&self, bytes: &[u8]) -> Result<PathBuf> {
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
                .join(format!("models-codex.json.bak-{stamp}{suffix}"));
            if path.exists() {
                continue;
            }
            files::create(&path, bytes)?;
            return Ok(path);
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
