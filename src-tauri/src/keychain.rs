//! OS キーチェーンに LLM API キーなどの機密情報を保管する薄いラッパー。
//!
//! - macOS: Keychain Services
//! - Windows: Credential Manager
//! - Linux: secret-service (libsecret)
//!
//! サービス名は tauri.conf.json の identifier と合わせて `com.lumencite.app`。
//! アカウント名は `llm.api_key.openai` / `llm.api_key.anthropic` のように
//! `<scope>.<name>` 形式。

use std::fmt;

const SERVICE: &str = "com.lumencite.app";

#[derive(Debug)]
pub struct KeychainError(pub String);

impl fmt::Display for KeychainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "keychain error: {}", self.0)
    }
}

impl std::error::Error for KeychainError {}

impl From<keyring::Error> for KeychainError {
    fn from(e: keyring::Error) -> Self {
        KeychainError(e.to_string())
    }
}

fn entry(account: &str) -> Result<keyring::Entry, KeychainError> {
    keyring::Entry::new(SERVICE, account).map_err(KeychainError::from)
}

pub fn set(account: &str, value: &str) -> Result<(), KeychainError> {
    entry(account)?.set_password(value).map_err(KeychainError::from)
}

pub fn get(account: &str) -> Result<Option<String>, KeychainError> {
    match entry(account)?.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(KeychainError::from(e)),
    }
}

pub fn delete(account: &str) -> Result<(), KeychainError> {
    match entry(account)?.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(KeychainError::from(e)),
    }
}

pub fn account_for_api_key(provider: &str) -> String {
    format!("llm.api_key.{}", provider)
}

/// MCP サーバー（公開）の Bearer 認可トークンのキーチェーンアカウント名。
pub fn account_for_mcp_token() -> String {
    "mcp_server.token".to_string()
}
