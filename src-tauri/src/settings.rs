use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Config {
    pub vault_path: Option<String>,
    /// Legado: API key em plaintext. Mantido só para MIGRAÇÃO — na primeira
    /// leitura é cifrado para `api_key_enc` e removido do arquivo.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// API key cifrada com DPAPI (CryptProtectData, escopo current-user),
    /// serializada em hex. Só é decifrável pelo mesmo usuário do Windows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_enc: Option<String>,
}

pub fn config_dir() -> anyhow::Result<PathBuf> {
    let dir = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("não foi possível obter o diretório de configuração"))?
        .join("Artemis");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn config_file() -> anyhow::Result<PathBuf> {
    Ok(config_dir()?.join("config.json"))
}

pub fn load_config() -> Config {
    config_file()
        .ok()
        .and_then(|p| fs::read_to_string(&p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_config(config: &Config) -> anyhow::Result<()> {
    let path = config_file()?;
    let json = serde_json::to_string_pretty(config)?;
    fs::write(&path, json)?;
    Ok(())
}

// API key: cifrada com DPAPI (CryptProtectData) dentro do config.json.
//
// Histórico: a primeira versão usava o crate `keyring` (Credential Manager),
// mas keyring 3.6.3 falha silenciosamente em alguns Windows 11: set_password
// retorna Ok mas a credencial nunca chega ao Vault (gotcha 1 do HANDOFF).
// A segunda versão gravava plaintext em config.json. Agora usamos DPAPI direto
// via `windows-sys` (FFI fina, sem o peso do crate `windows` completo):
// - `CRYPTPROTECT_UI_FORBIDDEN` — nunca abre prompt de UI
// - escopo current-user: só o mesmo usuário do Windows decifra
// - blob serializado em hex no campo `api_key_enc`
// - migração transparente: plaintext legado em `api_key` é cifrado na primeira
//   leitura e removido do arquivo
// - lição do gotcha 1: TODO write faz roundtrip verify (cifra → decifra →
//   compara) antes de persistir.

/// Cifra bytes com DPAPI no escopo do usuário atual.
#[cfg(windows)]
fn dpapi_protect(data: &[u8]) -> anyhow::Result<Vec<u8>> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    let ok = unsafe {
        CryptProtectData(
            &input,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if ok == 0 || output.pbData.is_null() {
        anyhow::bail!("CryptProtectData falhou");
    }
    let bytes =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
    unsafe { LocalFree(output.pbData as *mut core::ffi::c_void) };
    Ok(bytes)
}

/// Decifra bytes cifrados por `dpapi_protect` (mesmo usuário do Windows).
#[cfg(windows)]
fn dpapi_unprotect(data: &[u8]) -> anyhow::Result<Vec<u8>> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    let ok = unsafe {
        CryptUnprotectData(
            &input,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if ok == 0 || output.pbData.is_null() {
        anyhow::bail!("CryptUnprotectData falhou (blob de outro usuário/máquina?)");
    }
    let bytes =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
    unsafe { LocalFree(output.pbData as *mut core::ffi::c_void) };
    Ok(bytes)
}

/// Fallback para builds não-Windows (CI eventual): pass-through sem cifra.
/// O app é Windows-only; isto existe só para o código compilar em outras
/// plataformas sem cfg spraying nos call-sites.
#[cfg(not(windows))]
fn dpapi_protect(data: &[u8]) -> anyhow::Result<Vec<u8>> {
    Ok(data.to_vec())
}

#[cfg(not(windows))]
fn dpapi_unprotect(data: &[u8]) -> anyhow::Result<Vec<u8>> {
    Ok(data.to_vec())
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{:02x}", b);
    }
    s
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

pub fn save_api_key(key: &str) -> anyhow::Result<()> {
    tracing::info!("save_api_key: cifrando com DPAPI (len={})", key.len());
    let blob = dpapi_protect(key.as_bytes())?;
    // Roundtrip verify (lição do gotcha 1: nunca assumir sucesso sem conferir)
    let back = dpapi_unprotect(&blob)?;
    if back != key.as_bytes() {
        anyhow::bail!("verificação DPAPI falhou: decifrado difere do original");
    }
    let mut config = load_config();
    config.api_key = None; // remove plaintext legado, se existir
    config.api_key_enc = Some(hex_encode(&blob));
    save_config(&config)?;
    tracing::info!("save_api_key: persistido cifrado");
    Ok(())
}

pub fn load_api_key() -> anyhow::Result<Option<String>> {
    let config = load_config();

    if let Some(enc) = &config.api_key_enc {
        match hex_decode(enc)
            .ok_or_else(|| anyhow::anyhow!("api_key_enc não é hex válido"))
            .and_then(|blob| dpapi_unprotect(&blob))
            .and_then(|bytes| Ok(String::from_utf8(bytes)?))
        {
            Ok(key) => {
                tracing::info!("load_api_key: decifrado (len={})", key.len());
                return Ok(Some(key));
            }
            Err(e) => {
                // Blob corrompido ou de outro usuário/máquina — usuário
                // precisa reconfigurar a chave nas Configurações.
                tracing::warn!("load_api_key: falha ao decifrar ({}); chave ignorada", e);
                return Ok(None);
            }
        }
    }

    // Migração: plaintext legado → cifrado. Se a gravação falhar, ainda
    // devolve a chave (funcionalidade > migração).
    if let Some(plain) = config.api_key.clone() {
        tracing::info!("load_api_key: migrando chave plaintext legada para DPAPI");
        if let Err(e) = save_api_key(&plain) {
            tracing::warn!("load_api_key: migração falhou ({}); usando plaintext", e);
        }
        return Ok(Some(plain));
    }

    tracing::warn!("load_api_key: ausente");
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip() {
        let data = [0u8, 1, 15, 16, 127, 128, 255];
        let hex = hex_encode(&data);
        assert_eq!(hex, "00010f10 7f80ff".replace(' ', ""));
        assert_eq!(hex_decode(&hex).unwrap(), data);
    }

    #[test]
    fn hex_decode_rejects_invalid() {
        assert!(hex_decode("0").is_none()); // ímpar
        assert!(hex_decode("zz").is_none()); // não-hex
        assert_eq!(hex_decode("").unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn dpapi_roundtrip() {
        // No Windows exercita CryptProtectData de verdade; em outras
        // plataformas o fallback pass-through também satisfaz a propriedade.
        let secret = "sk-test-1234567890";
        let blob = dpapi_protect(secret.as_bytes()).unwrap();
        let back = dpapi_unprotect(&blob).unwrap();
        assert_eq!(back, secret.as_bytes());
    }
}
