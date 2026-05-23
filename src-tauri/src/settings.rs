use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Config {
    pub vault_path: Option<String>,
    pub api_key: Option<String>,
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

// API key: armazenada em config.json (plaintext).
//
// Tentamos usar o crate `keyring` (Windows Credential Manager / DPAPI) na primeira
// versão, mas keyring 3.6.3 falha silenciosamente em alguns sistemas Windows 11:
// set_password retorna Ok mas a credencial nunca chega ao Vault. Como `cmdkey`
// (Wincred API direto) funciona normalmente, é bug específico da crate.
//
// O config.json fica em %APPDATA%/Artemis/config.json. Qualquer processo rodando
// como o mesmo usuário já teria acesso ao keyring também, então o threat model é
// equivalente para uma app desktop single-user. Futuro: cifrar com DPAPI direto via
// crate `windows` (CryptProtectData / CryptUnprotectData).

pub fn save_api_key(key: &str) -> anyhow::Result<()> {
    tracing::info!("save_api_key: gravando em config.json (len={})", key.len());
    let mut config = load_config();
    config.api_key = Some(key.to_string());
    save_config(&config)?;
    tracing::info!("save_api_key: persistido");
    Ok(())
}

pub fn load_api_key() -> anyhow::Result<Option<String>> {
    let key = load_config().api_key;
    match &key {
        Some(k) => tracing::info!("load_api_key: encontrado (len={})", k.len()),
        None => tracing::warn!("load_api_key: ausente"),
    }
    Ok(key)
}
