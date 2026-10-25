//! Isolated filesystem tests. All credentials below are synthetic.

use super::*;
use tempfile::TempDir;

const API: &str = r#"# shared settings
model_provider = "OpenAI"
model = "test-api-model"
model_reasoning_effort = "xhigh"
model_context_window = 372000
disable_response_storage = true
[model_providers."OpenAI"] # quoted header
name = "API gateway"
base_url = "https://gateway.example.invalid/v1" # preserve endpoint
wire_api = "responses"
requires_openai_auth = true
supports_websockets = true
[mcp_servers.current]
command = "keep-current-command"
[features]
multi_agent = true
[agents]
max_threads = 4
[projects."F:/project/example"]
trust_level = "trusted"
"#;
const SUBSCRIPTION: &str = r#"# subscription settings
model_provider = 'openai'
model = "test-subscription-model"
model_reasoning_effort = "max"
model_context_window = 375000
service_tier = "default"
personality = "pragmatic"
[mcp_servers.legacy]
command = "old-command"
[shell_environment_policy.set]
UNRELATED_AUTH_TOKEN = "synthetic-legacy-value"
"#;
const SUB_AUTH: &[u8] = br#"{"auth_mode":"chatgpt","OPENAI_API_KEY":null,"tokens":{"access_token":"synthetic-access","refresh_token":"synthetic-refresh","id_token":"synthetic-id","account_id":"synthetic-account"}}"#;
const IMPORTED_SUBSCRIPTION: &str = r#"model_provider = "openai"
model = "imported-subscription-model"
model_reasoning_effort = "high"
"#;
const IMPORTED_SUBSCRIPTION_AUTH: &[u8] = br#"{"auth_mode":"chatgpt","tokens":{"access_token":"synthetic-imported-access","refresh_token":"synthetic-imported-refresh","id_token":"synthetic-imported-id","account_id":"synthetic-imported-account"}}"#;
const KEY: &str = "synthetic-api-test-value";

struct Data {
    _temporary: TempDir,
    store: Store,
}
impl Data {
    fn new() -> anyhow::Result<Self> {
        let temporary = tempfile::tempdir()?;
        let paths = Paths {
            home: temporary.path().into(),
        };
        fs::write(paths.config(), API)?;
        fs::write(paths.auth(), config::api_auth(KEY)?)?;
        fs::write(
            paths.home.join("config.toml.bak-20261024-170412"),
            SUBSCRIPTION,
        )?;
        fs::write(paths.home.join("auth.json.bak"), SUB_AUTH)?;
        Ok(Self {
            _temporary: temporary,
            store: Store::isolated(paths),
        })
    }
    fn initialize(&self) -> Result<Status> {
        self.store.initialize(ImportSources::default())
    }
    fn active(&self) -> Result<Active> {
        self.store.active()
    }
    fn switch(&self, mode: Mode) -> Result<SwitchResult> {
        self.store
            .apply(self.store.prepare(mode, SwitchOptions::default())?)
    }
}

#[test]
fn initialize_imports_matching_backups_without_changing_active_files() -> anyhow::Result<()> {
    let data = Data::new()?;
    let before = data.active()?;
    let status = data.initialize()?;
    assert!(before == data.active()?);
    assert!(status.modes.iter().all(|m| m.ready));
    assert_eq!(
        status.modes[0].config_source,
        "config.toml.bak-20261024-170412"
    );
    let raw = fs::read(data.store.paths.vault())?;
    assert!(!raw.windows(KEY.len()).any(|v| v == KEY.as_bytes()));
    let plain = Cipher::open(&data.store.paths, false)?.decrypt("modes", &raw)?;
    assert!(!String::from_utf8_lossy(&plain).contains("synthetic-legacy-value"));
    Ok(())
}

#[test]
fn round_trip_restores_model_auth_and_preserves_shared_settings() -> anyhow::Result<()> {
    let data = Data::new()?;
    data.initialize()?;
    data.switch(Mode::Subscription)?;
    let active = data.active()?;
    let doc = config::parse(&active.config)?;
    assert_eq!(doc["model"].as_str(), Some("test-subscription-model"));
    assert_eq!(doc["model_reasoning_effort"].as_str(), Some("max"));
    assert_eq!(doc["model_context_window"].as_integer(), Some(375000));
    assert!(doc.get("disable_response_storage").is_none());
    assert_eq!(active.auth.as_deref(), Some(SUB_AUTH));
    assert!(doc.get("shell_environment_policy").is_none());
    for field in ["mcp_servers", "features", "agents", "projects"] {
        assert_eq!(
            doc[field].to_string(),
            config::parse(API.as_bytes())?[field].to_string()
        );
    }
    data.switch(Mode::Api)?;
    let active = data.active()?;
    let doc = config::parse(&active.config)?;
    assert_eq!(doc["model"].as_str(), Some("test-api-model"));
    assert!(doc.get("service_tier").is_none());
    assert!(doc.get("personality").is_none());
    assert!(String::from_utf8_lossy(&active.config).contains("# preserve endpoint"));
    assert_eq!(config::auth_mode(active.auth.as_deref())?, Some(Mode::Api));
    assert!(!active.config.contains(&b'\r'));
    Ok(())
}

#[test]
fn same_mode_is_noop_without_backup() -> anyhow::Result<()> {
    let data = Data::new()?;
    data.initialize()?;
    let before = data.active()?;
    let result = data.switch(Mode::Api)?;
    assert!(!result.changed);
    assert!(result.backup.is_none());
    assert!(before == data.active()?);
    Ok(())
}

#[test]
fn leaving_mode_saves_refreshed_tokens_and_user_edits() -> anyhow::Result<()> {
    let data = Data::new()?;
    data.initialize()?;
    data.switch(Mode::Subscription)?;
    let changed = fs::read_to_string(data.store.paths.config())?.replace("\"max\"", "\"high\"");
    fs::write(data.store.paths.config(), changed)?;
    let changed_auth =
        String::from_utf8(SUB_AUTH.into())?.replace("synthetic-refresh", "synthetic-refreshed");
    fs::write(data.store.paths.auth(), &changed_auth)?;
    data.switch(Mode::Api)?;
    data.switch(Mode::Subscription)?;
    assert_eq!(data.store.status()?.effort, "high");
    assert_eq!(fs::read_to_string(data.store.paths.auth())?, changed_auth);
    Ok(())
}

#[test]
fn imported_profile_survives_leaving_the_same_active_mode() -> anyhow::Result<()> {
    let data = Data::new()?;
    data.initialize()?;
    data.switch(Mode::Subscription)?;

    let imported_config = data.store.paths.home.join("imported-subscription.toml");
    let imported_auth = data
        .store
        .paths
        .home
        .join("imported-subscription-auth.json");
    fs::write(&imported_config, IMPORTED_SUBSCRIPTION)?;
    fs::write(&imported_auth, IMPORTED_SUBSCRIPTION_AUTH)?;
    let status = data.store.initialize(ImportSources {
        subscription_config: Some(imported_config),
        subscription_auth: Some(imported_auth),
        ..Default::default()
    })?;
    let imported = status
        .modes
        .iter()
        .find(|mode| mode.mode == Mode::Subscription)
        .ok_or_else(|| anyhow::anyhow!("subscription profile is missing"))?;
    assert_eq!(imported.model, "imported-subscription-model");

    data.switch(Mode::Api)?;
    data.switch(Mode::Subscription)?;
    let active = data.active()?;
    let config = config::parse(&active.config)?;
    assert_eq!(
        config["model"].as_str(),
        Some("imported-subscription-model")
    );
    assert_eq!(active.auth.as_deref(), Some(IMPORTED_SUBSCRIPTION_AUTH));
    Ok(())
}

#[test]
fn missing_target_auth_and_wrong_import_leave_active_unchanged() -> anyhow::Result<()> {
    let data = Data::new()?;
    fs::remove_file(data.store.paths.home.join("auth.json.bak"))?;
    data.initialize()?;
    let before = data.active()?;
    assert!(
        data.store
            .prepare(Mode::Subscription, SwitchOptions::default())
            .is_err()
    );
    let sources = ImportSources {
        subscription_auth: Some(data.store.paths.auth()),
        ..Default::default()
    };
    assert!(data.store.initialize(sources).is_err());
    assert!(before == data.active()?);
    Ok(())
}

#[test]
fn api_key_comes_from_input_or_saved_auth_but_other_environment_options_still_apply()
-> anyhow::Result<()> {
    let mut data = Data::new()?;
    data.store
        .environment
        .insert("AUTOCODE_API_KEY".into(), "synthetic-env-key".into());
    data.initialize()?;
    data.switch(Mode::Subscription)?;
    data.store
        .environment
        .insert("CODEX_API_PROVIDER".into(), "env.api".into());
    data.store.environment.insert(
        "AUTOCODE_BASE_URL".into(),
        "https://env.example.invalid/v1".into(),
    );
    let prepared = data.store.prepare(Mode::Api, SwitchOptions::default())?;
    assert_eq!(prepared.preview.auth_source, "latest active authentication");
    let result = data.store.apply(prepared)?;
    assert_eq!(result.environment_persisted, None);
    assert!(fs::read_to_string(data.store.paths.auth())?.contains(KEY));
    data.switch(Mode::Subscription)?;

    let options = SwitchOptions {
        provider: Some("ui.api".into()),
        base_url: Some("https://ui.example.invalid/v1".into()),
        api_key: Some(Zeroizing::new("synthetic-ui-key".into())),
    };
    let prepared = data.store.prepare(Mode::Api, options)?;
    assert_eq!(prepared.preview.auth_source, "user input");
    let result = data.store.apply(prepared)?;
    assert_eq!(result.environment_persisted, Some(true));
    let status = data.store.status()?;
    assert_eq!(status.provider, "ui.api");
    assert_eq!(status.base_url, "https://ui.example.invalid/v1");
    assert!(fs::read_to_string(data.store.paths.auth())?.contains("synthetic-ui-key"));
    data.switch(Mode::Subscription)?;
    let prepared = data.store.prepare(Mode::Api, SwitchOptions::default())?;
    assert_eq!(prepared.preview.auth_source, "latest active authentication");
    let result = data.store.apply(prepared)?;
    assert_eq!(result.environment_persisted, None);
    let status = data.store.status()?;
    assert_eq!(status.provider, "env.api");
    assert_eq!(status.base_url, "https://env.example.invalid/v1");
    assert!(fs::read_to_string(data.store.paths.auth())?.contains("synthetic-ui-key"));
    assert!(!fs::read_to_string(data.store.paths.auth())?.contains("synthetic-env-key"));
    assert!(!fs::read_to_string(data.store.paths.config())?.contains("synthetic-"));
    assert!(!data.store.paths.home.join(".env").exists());
    Ok(())
}

#[test]
fn subscription_start_works_without_profile_file() -> anyhow::Result<()> {
    let data = Data::new()?;
    fs::write(
        data.store.paths.config(),
        SUBSCRIPTION
            .split("[shell_environment_policy")
            .next()
            .unwrap_or(""),
    )?;
    fs::write(data.store.paths.auth(), SUB_AUTH)?;
    fs::remove_file(
        data.store
            .paths
            .home
            .join("config.toml.bak-20261024-170412"),
    )?;
    let status = data.initialize()?;
    assert_eq!(status.modes[1].config_source, "default API parameters");
    let options = SwitchOptions {
        api_key: Some(Zeroizing::new(KEY.into())),
        ..Default::default()
    };
    data.store.apply(data.store.prepare(Mode::Api, options)?)?;
    assert_eq!(
        data.store.status()?.base_url,
        "https://vip.auto-code.net/v1"
    );
    Ok(())
}

#[test]
fn invalid_toml_errors_hide_input_and_do_not_create_vault() -> anyhow::Result<()> {
    let data = Data::new()?;
    fs::write(data.store.paths.config(), "key = \"synthetic-secret")?;
    let error = data
        .initialize()
        .err()
        .ok_or_else(|| anyhow::anyhow!("expected error"))?;
    assert!(!error.to_string().contains("synthetic-secret"));
    assert!(!data.store.paths.vault().exists());
    Ok(())
}

#[test]
fn config_failure_rolls_back_auth_config_and_vault() -> anyhow::Result<()> {
    let mut data = Data::new()?;
    data.initialize()?;
    let before = data.active()?;
    let vault = fs::read(data.store.paths.vault())?;
    data.store.fail_after_auth = true;
    assert!(data.switch(Mode::Subscription).is_err());
    assert!(before == data.active()?);
    assert_eq!(vault, fs::read(data.store.paths.vault())?);
    assert!(!data.store.paths.journal().exists());
    Ok(())
}

fn interrupted_record(data: &Data) -> anyhow::Result<(Active, Cipher)> {
    let prepared = data
        .store
        .prepare(Mode::Subscription, SwitchOptions::default())?;
    let before = data.active()?;
    let cipher = Cipher::open(&data.store.paths, false)?;
    let record = Journal {
        version: 2,
        before: prepared.before,
        after: prepared.after,
        vault_before: prepared.vault_before,
        vault_after: Store::encode(&cipher, "modes", &prepared.state)?,
    };
    fs::write(
        data.store.paths.journal(),
        Store::encode(&cipher, "pending", &record)?,
    )?;
    data.store.restore_auth(record.after.auth.as_deref())?;
    Ok((before, cipher))
}

#[test]
fn encrypted_crash_record_recovers_on_next_initialize() -> anyhow::Result<()> {
    let data = Data::new()?;
    data.initialize()?;
    let (before, _) = interrupted_record(&data)?;
    assert!(before != data.active()?);
    assert!(
        !fs::read(data.store.paths.journal())?
            .windows(17)
            .any(|v| v == b"synthetic-refresh")
    );
    data.initialize()?;
    assert!(before == data.active()?);
    assert!(!data.store.paths.journal().exists());
    Ok(())
}

#[test]
fn external_changes_stop_recovery_and_keep_record() -> anyhow::Result<()> {
    let data = Data::new()?;
    data.initialize()?;
    interrupted_record(&data)?;
    fs::write(
        data.store.paths.config(),
        format!("{API}\n# external edit\n"),
    )?;
    let external = data.active()?;
    assert!(data.initialize().is_err());
    assert!(external == data.active()?);
    assert!(data.store.paths.journal().exists());
    Ok(())
}

#[test]
fn stale_preview_never_overwrites_external_changes() -> anyhow::Result<()> {
    let data = Data::new()?;
    data.initialize()?;
    let prepared = data
        .store
        .prepare(Mode::Subscription, SwitchOptions::default())?;
    fs::write(
        data.store.paths.config(),
        format!("{API}\n# external edit\n"),
    )?;
    let external = data.active()?;
    assert!(data.store.apply(prepared).is_err());
    assert!(external == data.active()?);
    assert!(!data.store.paths.journal().exists());
    Ok(())
}

#[test]
fn plaintext_secret_in_current_config_prevents_backups() -> anyhow::Result<()> {
    let data = Data::new()?;
    fs::write(
        data.store.paths.config(),
        format!("{API}\n[extra]\napi_key = \"synthetic-other-secret\"\n"),
    )?;
    data.initialize()?;
    let before = data.active()?;
    assert!(
        data.store
            .prepare(Mode::Subscription, SwitchOptions::default())
            .is_err()
    );
    assert!(before == data.active()?);
    Ok(())
}

#[test]
fn concurrent_operation_is_rejected() -> anyhow::Result<()> {
    let data = Data::new()?;
    let _lock = Lock::acquire(&data.store.paths)?;
    assert!(data.initialize().is_err());
    Ok(())
}

#[test]
fn missing_key_and_tampered_ciphertext_are_not_replaced() -> anyhow::Result<()> {
    let data = Data::new()?;
    data.initialize()?;
    let vault = fs::read(data.store.paths.vault())?;
    let key = fs::read(data.store.paths.key())?;
    fs::remove_file(data.store.paths.key())?;
    assert!(data.initialize().is_err());
    assert!(!data.store.paths.key().exists());
    assert_eq!(vault, fs::read(data.store.paths.vault())?);
    fs::write(data.store.paths.key(), key)?;
    let mut corrupted = vault;
    corrupted[20] ^= 1;
    fs::write(data.store.paths.vault(), &corrupted)?;
    assert!(data.initialize().is_err());
    assert_eq!(corrupted, fs::read(data.store.paths.vault())?);
    Ok(())
}

#[test]
fn encryption_uses_nonces_and_authenticated_purpose() -> anyhow::Result<()> {
    let data = Data::new()?;
    data.initialize()?;
    let cipher = Cipher::open(&data.store.paths, false)?;
    let first = cipher.encrypt("modes", b"synthetic-plain")?;
    let second = cipher.encrypt("modes", b"synthetic-plain")?;
    assert_ne!(first, second);
    assert!(cipher.decrypt("pending", &first).is_err());
    assert_eq!(&*cipher.decrypt("modes", &first)?, b"synthetic-plain");
    assert!(cipher.decrypt("modes", b"short").is_err());
    Ok(())
}

#[test]
fn reasoning_effort_is_checked_before_any_write() -> anyhow::Result<()> {
    let data = Data::new()?;
    data.initialize()?;
    fs::write(
        data.store.paths.home.join("models_cache.json"),
        r#"{"models":[{"slug":"test-subscription-model","supported_reasoning_levels":[{"effort":"high"}]}]}"#,
    )?;
    let before = data.active()?;
    assert!(
        data.store
            .prepare(Mode::Subscription, SwitchOptions::default())
            .is_err()
    );
    assert!(before == data.active()?);
    Ok(())
}

#[test]
fn unsafe_input_errors_are_redacted() {
    for url in [
        "https://user:synthetic-secret@example.invalid",
        "https://example.invalid?key=synthetic-secret",
        "ftp://example.invalid",
        "https:///v1",
        "https://example.invalid/\n",
    ] {
        assert!(config::endpoint(url).is_err());
    }
    for auth in [
        b"[1]".as_slice(),
        b"{\"OPENAI_API_KEY\":\"synthetic-secret".as_slice(),
    ] {
        let error = config::auth_mode(Some(auth)).err();
        assert!(error.is_some());
        assert!(
            !error
                .map(|e| e.to_string())
                .unwrap_or_default()
                .contains("synthetic-secret")
        );
    }
    assert!(config::api_auth("invalid\nvalue").is_err());
}

#[test]
fn bounded_reads_and_legacy_pending_stop_operations() -> anyhow::Result<()> {
    let data = Data::new()?;
    assert!(files::read(&data.store.paths.config(), 8).is_err());
    fs::create_dir_all(data.store.paths.home.join("auth-switcher"))?;
    fs::write(
        data.store.paths.home.join("auth-switcher/pending.dpapi"),
        b"synthetic-pending",
    )?;
    assert!(data.initialize().is_err());
    assert!(!data.store.paths.vault().exists());
    Ok(())
}
