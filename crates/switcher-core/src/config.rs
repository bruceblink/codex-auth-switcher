//! Mode-specific TOML fields and authentication structure checks. Never report parser input.

use crate::{Error, Mode, Result};
use serde_json::Value as Json;
use toml_edit::{DocumentMut, Item, Table, value};

pub const DEFAULT_API_BASE_URL: &str = "https://vip.auto-code.net/v1";

pub const MODE_FIELDS: &[&str] = &[
    "model_provider",
    "model",
    "model_reasoning_effort",
    "plan_mode_reasoning_effort",
    "model_reasoning_summary",
    "model_supports_reasoning_summaries",
    "model_context_window",
    "model_auto_compact_token_limit",
    "model_catalog_json",
    "disable_response_storage",
    "service_tier",
    "personality",
    "forced_login_method",
    "forced_chatgpt_workspace_id",
];

pub fn parse(bytes: &[u8]) -> Result<DocumentMut> {
    let text = std::str::from_utf8(bytes).map_err(|_| Error("Configuration must be UTF-8."))?;
    text.trim_start_matches('\u{feff}')
        .parse()
        .map_err(|_| Error("Invalid TOML configuration. Original files were not changed."))
}

pub fn provider(doc: &DocumentMut) -> Result<&str> {
    let name = match doc.get("model_provider") {
        Some(item) => item
            .as_str()
            .ok_or(Error("Provider ID must be a string."))?,
        None => "openai",
    };
    validate_provider(name)?;
    Ok(name)
}

pub fn validate_provider(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 128
        || !name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
    {
        return Err(Error(
            "Provider ID must contain only letters, digits, dots, dashes or underscores.",
        ));
    }
    Ok(())
}

pub fn endpoint(url: &str) -> Result<()> {
    let authority = url.split_once("://").map(|(_, tail)| tail).unwrap_or("");
    let parsed = url::Url::parse(url).map_err(|_| Error("Invalid API base URL."))?;
    if authority.is_empty()
        || authority.starts_with('/')
        || authority.starts_with('\\')
        || url.len() > 2048
        || !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || url.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(Error(
            "API base URL must be HTTP(S) with a host and no credentials, query or fragment.",
        ));
    }
    Ok(())
}

pub fn table(doc: &DocumentMut) -> Result<Option<&Table>> {
    let Some(providers) = doc.get("model_providers") else {
        return Ok(None);
    };
    let providers = providers
        .as_table()
        .ok_or(Error("Providers must be TOML tables."))?;
    providers
        .get(provider(doc)?)
        .map(|item| {
            item.as_table()
                .ok_or(Error("Selected provider must be a table."))
        })
        .transpose()
}

pub fn fragment(doc: &DocumentMut) -> Result<String> {
    let mut result = DocumentMut::new();
    for field in MODE_FIELDS {
        if let Some(item) = doc.get(field) {
            result[field] = item.clone();
        }
    }
    result["model_provider"] = value(provider(doc)?);
    if let Some(selected) = table(doc)? {
        let mut providers = Table::new();
        providers.insert(provider(doc)?, Item::Table(selected.clone()));
        result["model_providers"] = Item::Table(providers);
    }
    Ok(result.to_string())
}

pub fn apply(current: &[u8], snapshot: &str) -> Result<Vec<u8>> {
    let mut document = parse(current)?;
    let target = parse(snapshot.as_bytes())?;
    for field in MODE_FIELDS {
        match target.get(field) {
            Some(item) => {
                document[field] = item.clone();
            }
            None => {
                document.remove(field);
            }
        }
    }
    let name = provider(&target)?;
    if let Some(selected) = table(&target)? {
        if document.get("model_providers").is_none() {
            document["model_providers"] = Item::Table(Table::new());
        }
        document
            .get_mut("model_providers")
            .and_then(Item::as_table_mut)
            .ok_or(Error("Providers must be TOML tables."))?
            .insert(name, Item::Table(selected.clone()));
    } else if let Some(providers) = document
        .get_mut("model_providers")
        .and_then(Item::as_table_mut)
    {
        providers.remove(name);
    }
    let mut output = document
        .to_string()
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    if !output.ends_with('\n') {
        output.push('\n');
    }
    parse(output.as_bytes())?;
    Ok(output.into_bytes())
}

pub fn auth_mode(auth: Option<&[u8]>) -> Result<Option<Mode>> {
    let Some(bytes) = auth else {
        return Ok(None);
    };
    let json: Json = serde_json::from_slice(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes))
        .map_err(|_| Error("Invalid authentication JSON. Values are hidden."))?;
    if !json.is_object() {
        return Err(Error("Authentication must be a JSON object."));
    }
    let key = json
        .get("OPENAI_API_KEY")
        .and_then(Json::as_str)
        .is_some_and(|s| !s.trim().is_empty());
    let tokens = json
        .get("tokens")
        .and_then(Json::as_object)
        .is_some_and(|t| {
            ["access_token", "refresh_token", "id_token", "account_id"]
                .iter()
                .all(|k| {
                    t.get(*k)
                        .and_then(Json::as_str)
                        .is_some_and(|s| !s.is_empty())
                })
        });
    if key && tokens {
        return Err(Error(
            "Authentication contains conflicting API and subscription credentials.",
        ));
    }
    let kind = json.get("auth_mode").and_then(Json::as_str);
    match (key, tokens, kind) {
        (true, false, None | Some("api" | "apikey")) => Ok(Some(Mode::Api)),
        (false, true, None | Some("chatgpt")) => Ok(Some(Mode::Subscription)),
        _ => Ok(None),
    }
}

pub fn api_auth(key: &str) -> Result<Vec<u8>> {
    if key.is_empty()
        || key.len() > 16384
        || key.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(Error("API key is empty, too long or contains whitespace."));
    }
    serde_json::to_vec_pretty(&serde_json::json!({"OPENAI_API_KEY":key}))
        .map_err(|_| Error("Cannot encode API authentication."))
}

pub fn validate_profile(mode: Mode, snapshot: &str, auth: Option<&[u8]>) -> Result<()> {
    let doc = parse(snapshot.as_bytes())?;
    provider(&doc)?;
    if auth_mode(auth)? != Some(mode) {
        return Err(Error(
            "Target authentication is missing. Import a matching auth file.",
        ));
    }
    if doc
        .get("forced_login_method")
        .and_then(Item::as_str)
        .is_some_and(|s| s != if mode == Mode::Api { "api" } else { "chatgpt" })
    {
        return Err(Error("The forced login method conflicts with this mode."));
    }
    if has_credentials(doc.as_item()) {
        return Err(Error(
            "Provider TOML contains credentials. Use an auth file instead.",
        ));
    }
    if let Some(selected) = table(&doc)? {
        if let Some(url) = selected.get("base_url").and_then(Item::as_str) {
            endpoint(url)?;
        }
        if selected.get("env_key").is_some() {
            return Err(Error(
                "An environment-authenticated provider needs an API key before conversion.",
            ));
        }
        if selected.get("requires_openai_auth").and_then(Item::as_bool) == Some(false) {
            return Err(Error("Provider must use Codex file authentication."));
        }
    }
    if mode == Mode::Api {
        if matches!(provider(&doc)?, "openai" | "ollama" | "lmstudio") {
            return Err(Error(
                "API gateway must use a custom provider ID, such as OpenAI.",
            ));
        }
        let selected = table(&doc)?.ok_or(Error("The API provider table is missing."))?;
        endpoint(
            selected
                .get("base_url")
                .and_then(Item::as_str)
                .ok_or(Error("API base URL is missing."))?,
        )?;
        if selected.get("requires_openai_auth").and_then(Item::as_bool) != Some(true) {
            return Err(Error(
                "API provider must read authentication from auth.json.",
            ));
        }
        if selected.get("wire_api").and_then(Item::as_str) != Some("responses") {
            return Err(Error("The API provider must use the Responses API."));
        }
    }
    Ok(())
}

pub fn has_credentials(item: &Item) -> bool {
    if let Some(table) = item.as_table_like() {
        return table.iter().any(|(key, child)| {
            let key = key.to_ascii_lowercase();
            let sensitive = [
                "password",
                "secret",
                "token",
                "api_key",
                "apikey",
                "authorization",
                "cookie",
                "credential",
            ]
            .iter()
            .any(|s| key.contains(s));
            let exempt = [
                "limit",
                "window",
                "env_key",
                "instructions",
                "credentials_store",
            ]
            .iter()
            .any(|s| key.contains(s));
            (sensitive && !exempt && child.as_str().is_some_and(|s| !s.is_empty()))
                || has_credentials(child)
        });
    }
    if let Some(tables) = item.as_array_of_tables() {
        return tables
            .iter()
            .any(|t| has_credentials(&Item::Table(t.clone())));
    }
    false
}

pub fn summary(doc: &DocumentMut, field: &str) -> Result<String> {
    let text = doc.get(field).and_then(Item::as_str).unwrap_or("(default)");
    if text.len() > 160 || text.chars().any(char::is_control) {
        return Err(Error("Invalid model metadata."));
    }
    Ok(text.to_string())
}
