//! Best-effort checks and conservative cleanup for Codex `config.toml`.
//!
//! The checked field list is intentionally limited; it is not a complete Codex
//! schema validator. Deprecation guidance was checked against the official
//! reference on 2026-11-07:
//! <https://learn.chatgpt.com/docs/config-file/config-reference>.

use crate::{
    Error, Result, Store,
    files::{self, Lock},
};
use serde::Serialize;
#[cfg(test)]
use std::fs;
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item, Table, value};

const CONFIG_LIMIT: usize = files::CONFIG_LIMIT;

/// A read-only config report bound to one config file version and Codex directory.
#[derive(Clone, Serialize)]
pub struct ConfigCheck {
    pub path: PathBuf,
    pub issues: Vec<ConfigIssue>,
    #[serde(skip)]
    home: PathBuf,
    #[serde(skip)]
    original: Vec<u8>,
}

impl ConfigCheck {
    pub fn can_cleanup(&self) -> bool {
        self.issues.iter().any(|issue| issue.cleanup)
    }
}

/// A safe-to-display finding. Details never include parsed values or parser text.
#[derive(Clone, Serialize)]
pub struct ConfigIssue {
    pub key: String,
    pub title: String,
    pub detail: String,
    pub severity: ConfigIssueSeverity,
    pub cleanup: bool,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigIssueSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Serialize)]
pub struct ConfigCleanupResult {
    pub changed: bool,
    pub removed: Vec<String>,
    /// This backup contains only the removed settings, not the full config file.
    pub backup: Option<String>,
}

/// Check common fields and known deprecations without claiming full schema coverage.
pub fn check(store: &Store) -> Result<ConfigCheck> {
    let path = store.paths.config();
    let bytes = files::read(&path, CONFIG_LIMIT)?.ok_or(Error(
        "Codex config.toml was not found. Initialize the Codex configuration first.",
    ))?;
    let document = parse(&bytes)?;
    let issues = inspect(&document);
    Ok(ConfigCheck {
        path,
        issues,
        home: store.paths.home.clone(),
        original: bytes,
    })
}

/// Remove only settings confirmed to be ignored, preserving all other config data.
pub fn clean(store: &Store, check: &ConfigCheck) -> Result<ConfigCleanupResult> {
    if check.home != store.paths.home || check.path != store.paths.config() {
        return Err(Error(
            "The configuration check belongs to another Codex directory.",
        ));
    }

    let _lock = Lock::acquire(&store.paths)?;
    reject_pending_recovery(&store.paths.home, &store.paths.journal())?;

    let path = store.paths.config();
    let bytes = files::read(&path, CONFIG_LIMIT)?.ok_or(Error(
        "Codex config.toml was not found. Nothing was changed.",
    ))?;
    if bytes != check.original {
        return Err(Error(
            "Codex config.toml changed after it was checked. Check it again before cleanup.",
        ));
    }

    let mut document = parse(&bytes)?;
    let (removed, backup_patch) = remove_confirmed_ignored_settings(&mut document);
    if removed.is_empty() {
        return Ok(ConfigCleanupResult {
            changed: false,
            removed,
            backup: None,
        });
    }

    let updated = serialize(&document)?;
    // Recheck after creating the backup so an external edit is not overwritten.
    let current = files::read(&path, CONFIG_LIMIT)?.ok_or(Error(
        "Codex config.toml disappeared before cleanup. Check it again and retry.",
    ))?;
    if current != check.original {
        return Err(Error(
            "Codex config.toml changed during cleanup. Preserve the backup and check again.",
        ));
    }
    let backup = create_cleanup_backup(&path, &backup_patch)?;
    files::write(&path, &updated)?;

    Ok(ConfigCleanupResult {
        changed: true,
        removed,
        backup: Some(backup.to_string_lossy().into_owned()),
    })
}

fn parse(bytes: &[u8]) -> Result<DocumentMut> {
    let text = std::str::from_utf8(bytes).map_err(|_| Error("Configuration must be UTF-8."))?;
    text.trim_start_matches('\u{feff}')
        .parse()
        .map_err(|_| Error("Invalid TOML configuration. Original files were not changed."))
}

fn inspect(document: &DocumentMut) -> Vec<ConfigIssue> {
    let mut issues = Vec::new();

    for key in [
        "model_provider",
        "model",
        "personality",
        "model_catalog_json",
        "model_instructions_file",
        "experimental_instructions_file",
        "chatgpt_base_url",
        "plan_mode_reasoning_effort",
    ] {
        check_string(document, key, &mut issues);
    }
    for key in [
        "disable_response_storage",
        "model_supports_reasoning_summaries",
    ] {
        check_boolean(document, key, &mut issues);
    }
    for key in ["model_context_window", "model_auto_compact_token_limit"] {
        check_positive_integer(document, key, &mut issues);
    }
    check_percentage(
        document,
        "model_post_turn_compact_threshold_percent",
        &mut issues,
    );
    check_enum(
        document,
        "model_reasoning_effort",
        &[
            "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
        ],
        &mut issues,
    );
    check_enum(
        document,
        "model_verbosity",
        &["low", "medium", "high"],
        &mut issues,
    );
    check_enum(
        document,
        "forced_login_method",
        &["chatgpt", "api"],
        &mut issues,
    );
    check_approval_policy(document, &mut issues);
    check_enum(
        document,
        "sandbox_mode",
        &[
            "read-only",
            "workspace-write",
            "danger-full-access",
            "untrusted",
        ],
        &mut issues,
    );
    check_enum(
        document,
        "web_search",
        &["disabled", "cached", "live"],
        &mut issues,
    );

    if document.get("approval_policy").and_then(Item::as_str) == Some("on-failure") {
        issues.push(issue(
            "approval_policy",
            "审批策略需要检查",
            "on-failure 已过时。请按当前 Codex 规则选择策略。清理不会修改审批或安全设置。",
            ConfigIssueSeverity::Warning,
            false,
        ));
    } else if document.get("approval_policy").and_then(Item::as_str) == Some("untrusted") {
        issues.push(issue(
            "approval_policy",
            "审批策略不受支持",
            "untrusted 不受支持。请按当前 Codex 规则选择策略。清理不会修改审批或安全设置。",
            ConfigIssueSeverity::Warning,
            false,
        ));
    }
    if document.get("sandbox_mode").and_then(Item::as_str) == Some("untrusted") {
        issues.push(issue(
            "sandbox_mode",
            "沙箱模式不受支持",
            "untrusted 不受支持。请按当前 Codex 规则选择沙箱模式。清理不会修改沙箱设置。",
            ConfigIssueSeverity::Warning,
            false,
        ));
    }

    if document.get("experimental_instructions_file").is_some() {
        issues.push(issue(
            "experimental_instructions_file",
            "指令文件设置需要迁移",
            "请改用 model_instructions_file。检查器不会复制或删除此设置。",
            ConfigIssueSeverity::Warning,
            false,
        ));
    }

    inspect_features(document, &mut issues);

    if document
        .get("personality")
        .and_then(Item::as_str)
        .is_some_and(|personality| matches!(personality, "friendly" | "pragmatic"))
    {
        issues.push(issue(
            "personality",
            "个性值不再选择样式",
            "friendly 和 pragmatic 已过时。清理可以删除这两个值；none 仍有意义并会保留。",
            ConfigIssueSeverity::Warning,
            true,
        ));
    }

    issues
}

fn inspect_features(document: &DocumentMut, issues: &mut Vec<ConfigIssue>) {
    let Some(features) = document.get("features") else {
        return;
    };
    let Some(table) = features.as_table_like() else {
        issues.push(issue(
            "features",
            "功能设置类型无效",
            "此项必须是 TOML 表。检查器不会修改此值。",
            ConfigIssueSeverity::Error,
            false,
        ));
        return;
    };

    for key in ["personality", "unified_exec"] {
        if let Some(item) = table.get(key) {
            if item.as_bool().is_none() {
                issues.push(issue(
                    &format!("features.{key}"),
                    "功能设置类型无效",
                    "此项必须是布尔值。值类型异常时，清理不会删除此项。",
                    ConfigIssueSeverity::Error,
                    false,
                ));
            } else {
                issues.push(issue(
                    &format!("features.{key}"),
                    "功能设置已被忽略",
                    "此功能已移除或始终启用。清理可以删除此布尔设置。",
                    ConfigIssueSeverity::Info,
                    true,
                ));
            }
        }
    }

    for key in ["web_search", "web_search_cached", "web_search_request"] {
        if table.contains_key(key) {
            issues.push(issue(
                &format!("features.{key}"),
                "网页搜索设置需要迁移",
                "请先确认当前含义，再迁移到顶层 web_search。清理不会删除此设置。",
                ConfigIssueSeverity::Warning,
                false,
            ));
        }
    }
}

fn check_string(document: &DocumentMut, key: &str, issues: &mut Vec<ConfigIssue>) {
    let Some(item) = document.get(key) else {
        return;
    };
    if item.as_str().is_none() {
        issues.push(issue(
            key,
            "设置类型无效",
            "此项必须是字符串。检查器不会修改此值。",
            ConfigIssueSeverity::Error,
            false,
        ));
    }
}

fn check_boolean(document: &DocumentMut, key: &str, issues: &mut Vec<ConfigIssue>) {
    let Some(item) = document.get(key) else {
        return;
    };
    if item.as_bool().is_none() {
        issues.push(issue(
            key,
            "设置类型无效",
            "此项必须是布尔值。检查器不会修改此值。",
            ConfigIssueSeverity::Error,
            false,
        ));
    }
}

fn check_positive_integer(document: &DocumentMut, key: &str, issues: &mut Vec<ConfigIssue>) {
    let Some(item) = document.get(key) else {
        return;
    };
    if item.as_integer().is_none_or(|number| number <= 0) {
        issues.push(issue(
            key,
            "设置值无效",
            "此项必须是正整数。检查器不会修改此值。",
            ConfigIssueSeverity::Error,
            false,
        ));
    }
}

fn check_percentage(document: &DocumentMut, key: &str, issues: &mut Vec<ConfigIssue>) {
    let Some(item) = document.get(key) else {
        return;
    };
    if item
        .as_integer()
        .is_none_or(|number| !(0..=100).contains(&number))
    {
        issues.push(issue(
            key,
            "设置值无效",
            "此项必须是 0 到 100 的整数。检查器不会修改此值。",
            ConfigIssueSeverity::Error,
            false,
        ));
    }
}

fn check_enum(document: &DocumentMut, key: &str, allowed: &[&str], issues: &mut Vec<ConfigIssue>) {
    let Some(item) = document.get(key) else {
        return;
    };
    let Some(value) = item.as_str() else {
        issues.push(issue(
            key,
            "设置类型无效",
            "此项必须是受支持的字符串值。检查器不会修改此值。",
            ConfigIssueSeverity::Error,
            false,
        ));
        return;
    };
    if allowed.contains(&value) {
        return;
    }
    issues.push(issue(
        key,
        "设置值未识别",
        "此值不在检查器列出的选项中。请先查看当前 Codex 规则，再决定是否修改。",
        ConfigIssueSeverity::Warning,
        false,
    ));
}

fn check_approval_policy(document: &DocumentMut, issues: &mut Vec<ConfigIssue>) {
    let Some(item) = document.get("approval_policy") else {
        return;
    };
    if item.as_table_like().is_some() {
        return;
    }
    check_enum(
        document,
        "approval_policy",
        &["never", "on-request", "on-failure", "untrusted"],
        issues,
    );
}

fn issue(
    key: &str,
    title: &str,
    detail: &str,
    severity: ConfigIssueSeverity,
    cleanup: bool,
) -> ConfigIssue {
    ConfigIssue {
        key: key.to_string(),
        title: title.to_string(),
        detail: detail.to_string(),
        severity,
        cleanup,
    }
}

fn remove_confirmed_ignored_settings(document: &mut DocumentMut) -> (Vec<String>, DocumentMut) {
    let mut removed = Vec::new();
    let mut backup = DocumentMut::new();

    if let Some(features) = document
        .get_mut("features")
        .and_then(Item::as_table_like_mut)
    {
        for key in ["personality", "unified_exec"] {
            if let Some(item) = features.get(key)
                && item.as_bool().is_some()
            {
                let boolean = item.as_bool().unwrap_or(false);
                let backup_features = backup
                    .as_table_mut()
                    .entry("features")
                    .or_insert(Item::Table(Table::new()));
                if let Some(table) = backup_features.as_table_mut() {
                    table.insert(key, value(boolean));
                }
                features.remove(key);
                removed.push(format!("features.{key}"));
            }
        }
    }

    if let Some(personality) = document.get("personality").and_then(Item::as_str)
        && matches!(personality, "friendly" | "pragmatic")
    {
        backup["personality"] = value(personality);
        document.remove("personality");
        removed.push("personality".to_string());
    }

    (removed, backup)
}

fn serialize(document: &DocumentMut) -> Result<Vec<u8>> {
    let output = document
        .to_string()
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let mut output = output.into_bytes();
    if !output.ends_with(b"\n") {
        output.push(b'\n');
    }
    parse(&output)?;
    Ok(output)
}

fn create_cleanup_backup(config_path: &Path, patch: &DocumentMut) -> Result<PathBuf> {
    let home = config_path
        .parent()
        .ok_or(Error("Invalid configuration path."))?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let mut bytes = patch.to_string().replace("\r\n", "\n").into_bytes();
    if !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    for index in 0..1000 {
        let suffix = if index == 0 {
            String::new()
        } else {
            format!("-{index:02}")
        };
        let path = home.join(format!("config.toml.bak-cleanup-{stamp}{suffix}"));
        if path.exists() {
            continue;
        }
        files::create(&path, &bytes)?;
        return Ok(path);
    }
    Err(Error(
        "Cannot reserve a unique configuration cleanup backup.",
    ))
}

fn reject_pending_recovery(home: &Path, journal: &Path) -> Result<()> {
    if home.join("auth-switcher/pending.dpapi").exists() {
        return Err(Error(
            "Version one has a pending write. Recover it with version one before cleanup.",
        ));
    }
    if journal.exists() {
        return Err(Error(
            "Recovery is pending. Initialize modes to recover before cleanup.",
        ));
    }
    Ok(())
}

impl Store {
    pub fn check_config(&self) -> Result<ConfigCheck> {
        check(self)
    }

    pub fn clean_config(&self, check: &ConfigCheck) -> Result<ConfigCleanupResult> {
        clean(self, check)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Paths;
    use tempfile::TempDir;

    struct TestStore {
        _temporary: TempDir,
        store: Store,
    }

    impl TestStore {
        fn new(config: &str) -> anyhow::Result<Self> {
            let temporary = tempfile::tempdir()?;
            let paths = Paths {
                home: temporary.path().into(),
            };
            fs::write(paths.config(), config)?;
            Ok(Self {
                _temporary: temporary,
                store: Store::isolated(paths),
            })
        }
    }

    #[test]
    fn check_reports_common_types_and_preserves_meaningful_none() -> anyhow::Result<()> {
        let data = TestStore::new(
            "model_context_window = \"not-an-integer\"\n\
             model_reasoning_effort = \"impossible\"\n\
             personality = \"none\"\n",
        )?;

        let report = data.store.check_config()?;

        assert!(
            report
                .issues
                .iter()
                .any(|issue| { issue.key == "model_context_window" && !issue.cleanup })
        );
        assert!(
            report
                .issues
                .iter()
                .any(|issue| { issue.key == "model_reasoning_effort" && !issue.cleanup })
        );
        assert!(!report.issues.iter().any(|issue| issue.key == "personality"));
        assert!(!report.can_cleanup());
        Ok(())
    }

    #[test]
    fn invalid_toml_error_does_not_include_sensitive_source_text() -> anyhow::Result<()> {
        let data = TestStore::new("secret_marker = \"synthetic-private-value\"\ninvalid = [\n")?;

        let error = data
            .store
            .check_config()
            .err()
            .ok_or_else(|| anyhow::anyhow!("invalid TOML was accepted"))?;

        assert_eq!(
            error.to_string(),
            "Invalid TOML configuration. Original files were not changed."
        );
        assert!(!error.to_string().contains("synthetic-private-value"));
        Ok(())
    }

    #[test]
    fn cleanup_removes_only_confirmed_values_and_backups_only_the_patch() -> anyhow::Result<()> {
        let data = TestStore::new(
            "# keep shared settings\n\
             model_provider = \"openai\"\n\
             model = \"synthetic-model\"\n\
             personality = \"friendly\"\n\
             [model_providers.openai]\n\
             base_url = \"https://example.invalid/v1\"\n\
             api_key = \"synthetic-provider-secret\"\n\
             [mcp_servers.keep]\n\
             command = \"synthetic-mcp-command\"\n\
             [features]\n\
             personality = true\n\
             unified_exec = false\n\
             multi_agent = true\n\
             [custom]\n\
             note = \"keep-me\"\n",
        )?;
        let report = data.store.check_config()?;
        assert!(report.can_cleanup());

        let result = data.store.clean_config(&report)?;

        assert!(result.changed);
        assert_eq!(
            result.removed,
            [
                "features.personality",
                "features.unified_exec",
                "personality"
            ]
        );
        let updated = fs::read_to_string(data.store.paths.config())?;
        assert!(updated.contains("model_provider = \"openai\""));
        assert!(updated.contains("multi_agent = true"));
        assert!(updated.contains("[mcp_servers.keep]"));
        assert!(updated.contains("command = \"synthetic-mcp-command\""));
        assert!(updated.contains("api_key = \"synthetic-provider-secret\""));
        assert!(updated.contains("note = \"keep-me\""));
        assert!(!updated.contains("personality ="));
        assert!(!updated.contains("unified_exec ="));

        let backup = result
            .backup
            .ok_or_else(|| anyhow::anyhow!("cleanup backup was not created"))?;
        let backup = fs::read_to_string(backup)?;
        assert!(backup.contains("personality = \"friendly\""));
        assert!(backup.contains("personality = true"));
        assert!(backup.contains("unified_exec = false"));
        assert!(!backup.contains("synthetic-provider-secret"));
        assert!(!backup.contains("synthetic-mcp-command"));
        Ok(())
    }

    #[test]
    fn invalid_cleanup_types_and_unknown_settings_are_preserved() -> anyhow::Result<()> {
        let data = TestStore::new(
            "personality = \"none\"\n\
             unknown_option = \"preserve-me\"\n\
             [features]\n\
             personality = \"extension-value\"\n\
             unified_exec = true\n\
             multi_agent = true\n",
        )?;
        let report = data.store.check_config()?;
        assert!(
            report
                .issues
                .iter()
                .any(|issue| { issue.key == "features.personality" && !issue.cleanup })
        );

        let result = data.store.clean_config(&report)?;

        assert_eq!(result.removed, ["features.unified_exec"]);
        let updated = fs::read_to_string(data.store.paths.config())?;
        assert!(updated.contains("personality = \"none\""));
        assert!(updated.contains("personality = \"extension-value\""));
        assert!(updated.contains("multi_agent = true"));
        assert!(updated.contains("unknown_option = \"preserve-me\""));
        Ok(())
    }

    #[test]
    fn cleanup_rejects_config_changed_after_check() -> anyhow::Result<()> {
        let data = TestStore::new("personality = \"friendly\"\n")?;
        let report = data.store.check_config()?;
        fs::write(data.store.paths.config(), "personality = \"none\"\n")?;

        let error = data
            .store
            .clean_config(&report)
            .err()
            .ok_or_else(|| anyhow::anyhow!("changed config was cleaned"))?;

        assert_eq!(
            error.to_string(),
            "Codex config.toml changed after it was checked. Check it again before cleanup."
        );
        assert_eq!(
            fs::read_to_string(data.store.paths.config())?,
            "personality = \"none\"\n"
        );
        Ok(())
    }

    #[test]
    fn cleanup_rejects_pending_recovery_without_writing() -> anyhow::Result<()> {
        let data = TestStore::new("personality = \"friendly\"\n")?;
        let report = data.store.check_config()?;
        fs::create_dir_all(data.store.paths.state())?;
        fs::write(data.store.paths.journal(), b"synthetic-pending-record")?;
        let original = fs::read(data.store.paths.config())?;

        let error = data
            .store
            .clean_config(&report)
            .err()
            .ok_or_else(|| anyhow::anyhow!("cleanup proceeded during recovery"))?;

        assert_eq!(
            error.to_string(),
            "Recovery is pending. Initialize modes to recover before cleanup."
        );
        assert_eq!(fs::read(data.store.paths.config())?, original);
        assert!(!fs::read_dir(&data.store.paths.home)?.any(|entry| {
            entry
                .map(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with("config.toml.bak-cleanup-")
                })
                .unwrap_or(false)
        }));
        Ok(())
    }

    #[test]
    fn missing_config_is_rejected_without_creating_files() -> anyhow::Result<()> {
        let temporary = tempfile::tempdir()?;
        let store = Store::isolated(Paths {
            home: temporary.path().into(),
        });

        let error = store
            .check_config()
            .err()
            .ok_or_else(|| anyhow::anyhow!("missing config was accepted"))?;

        assert_eq!(
            error.to_string(),
            "Codex config.toml was not found. Initialize the Codex configuration first."
        );
        assert!(!store.paths.config().exists());
        assert!(!store.paths.state().exists());
        Ok(())
    }

    #[test]
    fn cleanup_is_idempotent_after_rechecking() -> anyhow::Result<()> {
        let data = TestStore::new(
            "personality = \"pragmatic\"\n\
             [features]\n\
             unified_exec = true\n",
        )?;
        let first = data.store.check_config()?;
        let first_result = data.store.clean_config(&first)?;
        assert!(first_result.changed);

        let second = data.store.check_config()?;
        assert!(!second.can_cleanup());
        let second_result = data.store.clean_config(&second)?;
        assert!(!second_result.changed);
        assert!(second_result.removed.is_empty());
        assert!(second_result.backup.is_none());
        Ok(())
    }
}
