//! Synthetic data for native acceptance. Never imports host credentials.
use std::{fs, path::Path};

pub fn create(home: &Path) -> anyhow::Result<()> {
    if home.exists() {
        anyhow::bail!("Demo destination already exists; choose a new directory.");
    }
    fs::create_dir_all(home)?;
    fs::write(
        home.join("config.toml"),
        r#"model_provider = "openai"
model = "demo-subscription-model"
model_reasoning_effort = "high"
cli_auth_credentials_store = "file"
[features]
multi_agent = true
[projects."F:/example"]
trust_level = "trusted"
"#,
    )?;
    fs::write(home.join("auth.json"), br#"{"auth_mode":"chatgpt","tokens":{"access_token":"synthetic-access","refresh_token":"synthetic-refresh","id_token":"synthetic-id","account_id":"synthetic-account"}}"#)?;
    fs::write(
        home.join("config.toml.bak-demo-api"),
        r#"model_provider = "OpenAI"
model = "demo-api-model"
model_reasoning_effort = "xhigh"
[model_providers.OpenAI]
name = "Demo gateway"
base_url = "https://gateway.example.invalid/v1"
wire_api = "responses"
requires_openai_auth = true
"#,
    )?;
    fs::write(
        home.join("auth.json.bak-demo-api"),
        switcher_core::config::api_auth("synthetic-demo-api-key")?,
    )?;
    Ok(())
}
