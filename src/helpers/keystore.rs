//! Stable façade over the encrypted secrets vault.
//!
//! All persistent secrets — AI provider API keys and per-connection URI / SSH /
//! proxy passwords — go through this module. Storage is delegated to
//! [`crate::helpers::secrets_vault`], a single AES-GCM-encrypted file unlocked
//! with a user-chosen master password at app startup.
//!
//! The async [`Task`]-returning signatures are preserved so callers do not need
//! to change. Vault operations themselves are synchronous and fast (a few
//! milliseconds for a small file).

use anyhow::Result;
use gpui::{App, Task};
use uuid::Uuid;

use crate::helpers::secrets_vault;

fn provider_key(provider: &str) -> String {
    format!("ai.{provider}")
}

fn conn_key(id: Uuid, key: &str) -> String {
    format!("conn.{id}.{key}")
}

fn vault_set(key: String, value: String) -> Result<()> {
    let vault = secrets_vault::global().ok_or_else(|| anyhow::anyhow!("vault not initialised"))?;
    vault.lock().map_err(|_| anyhow::anyhow!("vault mutex poisoned"))?.set(key, value)
}

fn vault_get(key: &str) -> Result<Option<String>> {
    let Some(vault) = secrets_vault::global() else {
        return Ok(None);
    };
    let guard = vault.lock().map_err(|_| anyhow::anyhow!("vault mutex poisoned"))?;
    Ok(guard.get(key))
}

fn vault_delete(key: &str) -> Result<()> {
    let Some(vault) = secrets_vault::global() else {
        return Ok(());
    };
    vault.lock().map_err(|_| anyhow::anyhow!("vault mutex poisoned"))?.remove(key)
}

pub struct KeyStore;

impl KeyStore {
    pub fn write(cx: &App, provider: &str, api_key: &str) -> Task<Result<()>> {
        let key = provider_key(provider);
        let value = api_key.to_string();
        cx.spawn(async move |_cx| vault_set(key, value))
    }

    pub fn read(cx: &App, provider: &str) -> Task<Result<Option<String>>> {
        let key = provider_key(provider);
        cx.spawn(async move |_cx| vault_get(&key))
    }

    pub fn delete(cx: &App, provider: &str) -> Task<Result<()>> {
        let key = provider_key(provider);
        cx.spawn(async move |_cx| vault_delete(&key))
    }

    pub fn write_conn(cx: &App, id: Uuid, key: &str, secret: &str) -> Task<Result<()>> {
        let storage_key = conn_key(id, key);
        let value = secret.to_string();
        cx.spawn(async move |_cx| vault_set(storage_key, value))
    }

    pub fn read_conn(cx: &App, id: Uuid, key: &str) -> Task<Result<Option<String>>> {
        let storage_key = conn_key(id, key);
        cx.spawn(async move |_cx| vault_get(&storage_key))
    }

    pub fn delete_conn(cx: &App, id: Uuid, key: &str) -> Task<Result<()>> {
        let storage_key = conn_key(id, key);
        cx.spawn(async move |_cx| vault_delete(&storage_key))
    }
}
