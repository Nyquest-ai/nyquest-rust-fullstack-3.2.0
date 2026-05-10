//! Nyquest Security Module — Encrypted Key Vault
//! Uses AES-256-GCM encryption with PBKDF2 key derivation.

use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use pbkdf2::pbkdf2_hmac;
use rand::RngCore;
use sha2::Sha256;
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use tracing::{error, info, warn};

const PBKDF2_ITERATIONS: u32 = 600_000;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32; // AES-256

pub struct KeyVault {
    vault_path: PathBuf,
    #[allow(dead_code)]
    key_path: PathBuf,
    #[allow(dead_code)]
    salt_path: PathBuf,
    cipher: Option<Aes256Gcm>,
}

impl KeyVault {
    pub fn new(vault_path: &str) -> Self {
        let expanded = shellexpand::tilde(vault_path).to_string();
        let vault_path = PathBuf::from(&expanded);

        let dir = vault_path.parent().unwrap_or(&vault_path);
        let key_path = dir.join("vault.key");
        let salt_path = dir.join("vault.salt");

        // Ensure directory exists with restricted permissions
        if !dir.exists() {
            let _ = fs::create_dir_all(dir);
            let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
        }

        let cipher = Self::init_cipher(&salt_path, &key_path);

        Self {
            vault_path,
            key_path,
            salt_path,
            cipher,
        }
    }

    fn init_cipher(salt_path: &PathBuf, key_path: &PathBuf) -> Option<Aes256Gcm> {
        // Try password from env first
        if let Ok(password) = std::env::var("NYQUEST_VAULT_PASSWORD") {
            let salt = Self::get_or_create_salt(salt_path);
            let mut derived_key = [0u8; KEY_LEN];
            pbkdf2_hmac::<Sha256>(
                password.as_bytes(),
                &salt,
                PBKDF2_ITERATIONS,
                &mut derived_key,
            );
            return Some(Aes256Gcm::new_from_slice(&derived_key).unwrap());
        }

        // Fall back to auto-generated keyfile
        let raw_key = if key_path.exists() {
            match fs::read(key_path) {
                Ok(k) => k,
                Err(e) => {
                    error!("Failed to read vault key: {}", e);
                    return None;
                }
            }
        } else {
            let mut key = [0u8; KEY_LEN];
            OsRng.fill_bytes(&mut key);
            if let Err(e) = fs::write(key_path, key) {
                error!("Failed to write vault key: {}", e);
                return None;
            }
            let _ = fs::set_permissions(key_path, fs::Permissions::from_mode(0o600));
            info!("Generated vault key at {:?}", key_path);
            key.to_vec()
        };

        if raw_key.len() >= KEY_LEN {
            Some(Aes256Gcm::new_from_slice(&raw_key[..KEY_LEN]).unwrap())
        } else {
            warn!(
                "Vault key too short ({} bytes), encryption disabled",
                raw_key.len()
            );
            None
        }
    }

    fn get_or_create_salt(salt_path: &PathBuf) -> Vec<u8> {
        if salt_path.exists() {
            if let Ok(salt) = fs::read(salt_path) {
                return salt;
            }
        }
        let mut salt = vec![0u8; SALT_LEN];
        OsRng.fill_bytes(&mut salt);
        let _ = fs::write(salt_path, &salt);
        let _ = fs::set_permissions(salt_path, fs::Permissions::from_mode(0o600));
        salt
    }

    fn load_vault(&self) -> HashMap<String, String> {
        if !self.vault_path.exists() {
            return HashMap::new();
        }

        let raw = match fs::read(&self.vault_path) {
            Ok(r) => r,
            Err(e) => {
                error!("Failed to read vault: {}", e);
                return HashMap::new();
            }
        };

        if let Some(cipher) = &self.cipher {
            // Encrypted: format is base64(nonce || ciphertext)
            let decoded = match B64.decode(&raw) {
                Ok(d) => d,
                Err(_) => {
                    // Try reading as raw bytes (non-base64 format)
                    raw.clone()
                }
            };

            if decoded.len() < NONCE_LEN + 1 {
                error!("Vault data too short");
                return HashMap::new();
            }

            let (nonce_bytes, ciphertext) = decoded.split_at(NONCE_LEN);
            let nonce = Nonce::from_slice(nonce_bytes);

            match cipher.decrypt(nonce, ciphertext) {
                Ok(plaintext) => serde_json::from_slice(&plaintext).unwrap_or_default(),
                Err(e) => {
                    error!("Failed to decrypt vault: {}", e);
                    HashMap::new()
                }
            }
        } else {
            // Plaintext fallback
            serde_json::from_slice(&raw).unwrap_or_default()
        }
    }

    fn save_vault(&self, data: &HashMap<String, String>) {
        let payload = serde_json::to_vec_pretty(data).unwrap_or_default();

        let to_write = if let Some(cipher) = &self.cipher {
            let mut nonce_bytes = [0u8; NONCE_LEN];
            OsRng.fill_bytes(&mut nonce_bytes);
            let nonce = Nonce::from_slice(&nonce_bytes);

            match cipher.encrypt(nonce, payload.as_ref()) {
                Ok(ciphertext) => {
                    let mut combined = nonce_bytes.to_vec();
                    combined.extend(ciphertext);
                    B64.encode(&combined).into_bytes()
                }
                Err(e) => {
                    error!("Failed to encrypt vault: {}", e);
                    return;
                }
            }
        } else {
            payload
        };

        if let Err(e) = fs::write(&self.vault_path, &to_write) {
            error!("Failed to write vault: {}", e);
            return;
        }
        let _ = fs::set_permissions(&self.vault_path, fs::Permissions::from_mode(0o600));
    }

    pub fn store(&self, provider: &str, api_key: &str) {
        let mut vault = self.load_vault();
        vault.insert(provider.to_string(), api_key.to_string());
        self.save_vault(&vault);
        info!(
            "Stored API key for '{}' (encrypted={})",
            provider,
            self.cipher.is_some()
        );
    }

    pub fn retrieve(&self, provider: &str) -> Option<String> {
        self.load_vault().get(provider).cloned()
    }

    pub fn remove(&self, provider: &str) -> bool {
        let mut vault = self.load_vault();
        if vault.remove(provider).is_some() {
            self.save_vault(&vault);
            true
        } else {
            false
        }
    }

    pub fn list_providers(&self) -> Vec<String> {
        self.load_vault().keys().cloned().collect()
    }

    pub fn has_key(&self, provider: &str) -> bool {
        self.load_vault().contains_key(provider)
    }
}

/// Resolve API key with priority: headers → env → vault
pub fn resolve_api_key(
    provider: &str,
    headers: &HashMap<String, String>,
    vault: Option<&KeyVault>,
) -> Option<String> {
    // 1. Request headers
    if let Some(key) = headers.get("x-api-key") {
        if !key.is_empty() {
            return Some(key.clone());
        }
    }
    if let Some(auth) = headers.get("authorization") {
        if let Some(key) = auth.strip_prefix("Bearer ") {
            return Some(key.to_string());
        }
    }

    // 2. Environment variables
    let env_key = match provider {
        "anthropic" => "ANTHROPIC_API_KEY",
        "openai" => "OPENAI_API_KEY",
        "openrouter" => "OPENROUTER_API_KEY",
        "gemini" => "GEMINI_API_KEY",
        "xai" => "XAI_API_KEY",
        _ => return None,
    };
    if let Ok(val) = std::env::var(env_key) {
        if !val.is_empty() {
            return Some(val);
        }
    }

    // 3. Encrypted vault
    if let Some(v) = vault {
        return v.retrieve(provider);
    }

    None
}
