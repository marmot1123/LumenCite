//! 保存時暗号化（CR-012）。外部 MCP サーバーの `env` 秘密値などを SQLite の
//! `settings` テーブルに **平文で置かない**ための薄いラッパー。
//!
//! # 方式（単一マスター鍵）
//!
//! - OS keychain には 32byte のマスター鍵を **1 つだけ**置く（初回に生成）。
//! - 秘密値は AES-256-GCM でこのマスター鍵を使って暗号化し、
//!   `enc:v1:<base64(nonce ‖ ciphertext+tag)>` 形式の文字列で保存する。
//! - マスター鍵はプロセス内で [`OnceLock`] にキャッシュするため、keychain へ
//!   触るのは 1 プロセスにつき原則 1 回（＝プロンプトも 1 回）で済む。
//!
//! DB バックアップ・診断用 DB コピーが漏れても、keychain のマスター鍵が無ければ
//! 秘密値は復号できない。

use std::sync::OnceLock;

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::Engine;

use crate::keychain;

/// 暗号化文字列の版付きプレフィックス。既存の平文と区別し、将来の鍵/方式更新にも備える。
const ENC_PREFIX: &str = "enc:v1:";

/// マスター鍵の keychain アカウント名。
fn master_account() -> String {
    "secretbox.master_key".to_string()
}

/// プロセス内キャッシュ。keychain アクセス（＝プロンプト）を 1 回に抑える。
static MASTER_KEY: OnceLock<[u8; 32]> = OnceLock::new();

fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

/// マスター鍵を取得する。keychain に無ければ生成して保存する。
fn master_key() -> Result<&'static [u8; 32], String> {
    if let Some(k) = MASTER_KEY.get() {
        return Ok(k);
    }
    let account = master_account();
    let key: [u8; 32] = match keychain::get(&account).map_err(|e| e.to_string())? {
        Some(stored) => {
            let raw = b64()
                .decode(stored.as_bytes())
                .map_err(|e| format!("corrupt master key: {e}"))?;
            raw.try_into()
                .map_err(|_| "master key has unexpected length".to_string())?
        }
        None => {
            let generated = Aes256Gcm::generate_key(OsRng);
            let bytes: [u8; 32] = generated.into();
            keychain::set(&account, &b64().encode(bytes)).map_err(|e| e.to_string())?;
            bytes
        }
    };
    // 競合で別スレッドが先に set していてもどちらも同じ鍵（keychain 由来）なので問題ない。
    Ok(MASTER_KEY.get_or_init(|| key))
}

/// `s` が本モジュールで暗号化された文字列か。
pub fn is_encrypted(s: &str) -> bool {
    s.starts_with(ENC_PREFIX)
}

/// 平文を暗号化して `enc:v1:...` 文字列にする。空文字はそのまま（暗号化しない）。
pub fn encrypt(plaintext: &str) -> Result<String, String> {
    if plaintext.is_empty() {
        return Ok(String::new());
    }
    encrypt_with(master_key()?, plaintext)
}

/// `enc:v1:...` 文字列を復号する。暗号化されていない（平文の）入力はそのまま返す。
pub fn decrypt(s: &str) -> Result<String, String> {
    if !is_encrypted(s) {
        return Ok(s.to_string());
    }
    decrypt_with(master_key()?, s)
}

/// 鍵を明示して暗号化する（keychain を触らないのでテスト可能）。
fn encrypt_with(key: &[u8; 32], plaintext: &str) -> Result<String, String> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce = Aes256Gcm::generate_nonce(OsRng); // 96-bit
    let ct = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|e| format!("encrypt failed: {e}"))?;
    let mut blob = nonce.to_vec();
    blob.extend_from_slice(&ct);
    Ok(format!("{ENC_PREFIX}{}", b64().encode(blob)))
}

/// 鍵を明示して復号する（keychain を触らないのでテスト可能）。
fn decrypt_with(key: &[u8; 32], s: &str) -> Result<String, String> {
    let b64_part = s
        .strip_prefix(ENC_PREFIX)
        .ok_or_else(|| "not an encrypted value".to_string())?;
    let blob = b64()
        .decode(b64_part.as_bytes())
        .map_err(|e| format!("corrupt ciphertext: {e}"))?;
    if blob.len() < 12 {
        return Err("ciphertext too short".to_string());
    }
    let (nonce_bytes, ct) = blob.split_at(12);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let pt = cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ct)
        .map_err(|e| format!("decrypt failed: {e}"))?;
    String::from_utf8(pt).map_err(|e| format!("decrypted value is not utf-8: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // keychain を触るテストは環境依存（CI では secret-service が無いこともある）。
    // 純粋なフォーマット判定のみここで検証し、往復は #[ignore] にする。

    #[test]
    fn is_encrypted_detects_prefix() {
        assert!(is_encrypted("enc:v1:abc"));
        assert!(!is_encrypted("plain"));
        assert!(!is_encrypted(""));
    }

    #[test]
    fn decrypt_passthrough_for_plaintext() {
        // プレフィックスが無ければ平文として素通しする（migration 互換）。
        assert_eq!(decrypt("sk-plaintext").unwrap(), "sk-plaintext");
    }

    #[test]
    fn round_trip_with_fixed_key() {
        // keychain を触らず、鍵注入で AES-256-GCM の往復を検証する。
        let key = [7u8; 32];
        let secret = "super-secret-token";
        let enc = encrypt_with(&key, secret).unwrap();
        assert!(is_encrypted(&enc));
        assert_ne!(enc, secret);
        assert_eq!(decrypt_with(&key, &enc).unwrap(), secret);
    }

    #[test]
    fn nonce_is_randomized_per_encryption() {
        // 同じ平文・同じ鍵でも nonce が異なるため暗号文は毎回変わる。
        let key = [3u8; 32];
        let a = encrypt_with(&key, "same").unwrap();
        let b = encrypt_with(&key, "same").unwrap();
        assert_ne!(a, b);
        assert_eq!(decrypt_with(&key, &a).unwrap(), "same");
        assert_eq!(decrypt_with(&key, &b).unwrap(), "same");
    }

    #[test]
    fn wrong_key_fails_to_decrypt() {
        let enc = encrypt_with(&[1u8; 32], "secret").unwrap();
        assert!(decrypt_with(&[2u8; 32], &enc).is_err());
    }
}
