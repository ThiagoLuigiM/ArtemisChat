use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_updater::UpdaterExt;

use crate::deepseek::{self, ChatMessage};
use crate::history::{Entry, History};
use crate::prompt::PromptBuilder;
use crate::settings;
use crate::stats::{self, CamposSuggestions, PathSuggestion, ReleaseSuggestion};
use crate::vault::{self, VaultLoader, VaultStatus, Watcher};

pub struct VaultState {
    pub loader: Arc<RwLock<VaultLoader>>,
    pub watcher: Mutex<Option<Watcher>>,
}

pub struct HistoryState {
    pub history: Arc<History>,
}

#[tauri::command]
pub async fn open_chat(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("chat") {
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Stream do modo REVISÃO: aplica estilo.md/evitar.md/campos-padrao.md a uma
/// devolutiva escrita à mão, sem reescrever do zero. Reusa os eventos
/// `deepseek-token`/`deepseek-done` — a UI cai no ResultView normal e os
/// fluxos de aprovar/descartar funcionam iguais (raw_input = texto colado).
#[tauri::command]
pub async fn stream_revision(
    app: AppHandle,
    user_text: String,
    vault_state: State<'_, VaultState>,
) -> Result<(), String> {
    if user_text.trim().is_empty() {
        return Err("Cole o texto da devolutiva para revisar.".to_string());
    }
    let api_key = settings::load_api_key()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "API key da DeepSeek não configurada.".to_string())?;

    let messages = {
        let loader = vault_state.loader.read().unwrap();
        crate::prompt::build_revision_messages(loader.context(), &user_text)
    };

    let app_emit = app.clone();
    let result = deepseek::stream_chat(&api_key, messages, move |token| {
        if let Some(chat) = app_emit.get_webview_window("chat") {
            let _ = chat.emit(
                "deepseek-token",
                TokenEvent {
                    content: token.to_string(),
                },
            );
        }
    })
    .await;

    if let Some(chat) = app.get_webview_window("chat") {
        let _ = chat.emit("deepseek-done", ());
    }
    result.map_err(|e| e.to_string())
}

// ───────────────────────────────────────────────────────────────────────────────
// Cartilha HTML (#22) — gera prosa didática via IA + salva HTML+imagens no vault
// ───────────────────────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
pub struct CartilhaImageDto {
    /// Bytes brutos da imagem. Frontend envia como Uint8Array → Tauri serializa
    /// como `Vec<u8>` automaticamente (sem precisar base64).
    pub bytes: Vec<u8>,
    /// "png", "jpg", "jpeg", "webp", etc. Sanitizada pelo `save_cartilha`.
    pub extension: String,
    pub caption: String,
}

#[derive(Serialize, Clone)]
struct CartilhaTokenEvent {
    content: String,
}

/// Stream da geração de cartilha. Mesma mecânica do `stream_completion` (devolutiva)
/// mas com prompt didático e eventos separados: `cartilha-token` / `cartilha-done`.
/// Não bloqueia o stream principal — usuário pode gerar ambos em janelas separadas.
#[tauri::command]
pub async fn stream_cartilha(
    app: AppHandle,
    form_input: String,
    audience: String,
    image_captions: Vec<String>,
) -> Result<(), String> {
    let api_key = settings::load_api_key()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "API key da DeepSeek não configurada.".to_string())?;

    let messages = deepseek::build_cartilha_messages(&form_input, &audience, &image_captions);

    let app_emit = app.clone();
    let result = deepseek::stream_chat(&api_key, messages, move |token| {
        if let Some(chat) = app_emit.get_webview_window("chat") {
            let _ = chat.emit(
                "cartilha-token",
                CartilhaTokenEvent {
                    content: token.to_string(),
                },
            );
        }
    })
    .await;

    if let Some(chat) = app.get_webview_window("chat") {
        let _ = chat.emit("cartilha-done", ());
    }
    result.map_err(|e| e.to_string())
}

/// Salva a cartilha aprovada pelo usuário no vault. Cria pasta
/// `cartilhas/YYYY-MM-DD-<slug-titulo>/` com `index.html` + `imagens/NN.ext`.
/// Retorna o path absoluto do `index.html` gerado.
#[tauri::command]
pub fn save_cartilha(
    title: String,
    content: String,
    release: Option<String>,
    author: Option<String>,
    images: Vec<CartilhaImageDto>,
    vault_state: State<'_, VaultState>,
) -> Result<String, String> {
    let vault_path = vault_state
        .loader
        .read()
        .unwrap()
        .status()
        .path
        .clone()
        .ok_or_else(|| "Vault não configurado — selecione a pasta nas Configurações.".to_string())?;

    let image_inputs: Vec<vault::CartilhaImageInput> = images
        .iter()
        .map(|img| vault::CartilhaImageInput {
            bytes: &img.bytes,
            extension: &img.extension,
            caption: &img.caption,
        })
        .collect();

    let path = PathBuf::from(&vault_path);
    let index = vault::save_cartilha(
        &path,
        &title,
        &content,
        release.as_deref(),
        author.as_deref(),
        &image_inputs,
    )
    .map_err(|e| e.to_string())?;

    tracing::info!(
        "cartilha salva: {} ({} imagens)",
        index.display(),
        image_inputs.len()
    );
    maybe_git_backup(&path);

    Ok(index.to_string_lossy().into_owned())
}

/// Renderiza a cartilha numa pasta temporária (fora do vault) pro usuário
/// pré-visualizar o HTML final no navegador antes de salvar. Não exige vault
/// configurado; a pasta de preview é recriada a cada chamada.
#[tauri::command]
pub fn preview_cartilha(
    title: String,
    content: String,
    release: Option<String>,
    author: Option<String>,
    images: Vec<CartilhaImageDto>,
) -> Result<String, String> {
    let image_inputs: Vec<vault::CartilhaImageInput> = images
        .iter()
        .map(|img| vault::CartilhaImageInput {
            bytes: &img.bytes,
            extension: &img.extension,
            caption: &img.caption,
        })
        .collect();

    let index = vault::preview_cartilha(
        &title,
        &content,
        release.as_deref(),
        author.as_deref(),
        &image_inputs,
    )
    .map_err(|e| e.to_string())?;

    tracing::info!("preview de cartilha gerado: {}", index.display());
    Ok(index.to_string_lossy().into_owned())
}

// ───────────────────────────────────────────────────────────────────────────────
// Form de testes (#23) — sugestões de cenários via IA + compilação do texto final
// ───────────────────────────────────────────────────────────────────────────────

/// Exporta a cartilha salva para PDF usando o Edge headless (`--print-to-pdf`).
/// Sem dependências novas: Edge/WebView2 já é pré-requisito do app no Windows.
/// O PDF fica em `cartilha.pdf` ao lado do `index.html`; o CSS `@media print`
/// do template (esconde sidebar, remove sombras) é aplicado pelo Chromium.
#[tauri::command]
pub async fn export_cartilha_pdf(index_html_path: String) -> Result<String, String> {
    let index = PathBuf::from(&index_html_path);
    if !index.exists() {
        return Err(format!("arquivo não encontrado: {}", index_html_path));
    }
    let pdf_path = index.with_file_name("cartilha.pdf");

    let edge = [
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|p| p.exists())
    .ok_or_else(|| "Microsoft Edge não encontrado para gerar o PDF.".to_string())?;

    let url = format!("file:///{}", index.to_string_lossy().replace('\\', "/"));
    let pdf_arg = format!("--print-to-pdf={}", pdf_path.to_string_lossy());
    let pdf_check = pdf_path.clone();

    let status = tauri::async_runtime::spawn_blocking(move || {
        std::process::Command::new(edge)
            .args(["--headless", "--disable-gpu", "--no-pdf-header-footer", &pdf_arg, &url])
            .status()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("falha ao executar o Edge: {}", e))?;

    if !status.success() || !pdf_check.exists() {
        return Err("Edge headless não gerou o PDF.".to_string());
    }

    tracing::info!("PDF da cartilha gerado: {}", pdf_check.display());
    Ok(pdf_check.to_string_lossy().into_owned())
}

/// Lê a hotkey global configurada (formato canônico, ex: "Ctrl+Shift+KeyD").
#[tauri::command]
pub fn get_hotkey() -> Result<String, String> {
    Ok(settings::load_hotkey())
}

/// Valida, re-registra em runtime e persiste uma nova hotkey global. Se o
/// registro do novo atalho falhar (ex: combinação em uso por outro app),
/// restaura o anterior e devolve erro.
#[tauri::command]
pub fn set_hotkey(app: AppHandle, hotkey: String) -> Result<(), String> {
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

    let trimmed = hotkey.trim().to_string();
    let new_shortcut: Shortcut = trimmed
        .parse()
        .map_err(|e| format!("Atalho inválido '{}': {:?}", trimmed, e))?;

    let previous = settings::load_hotkey();
    app.global_shortcut()
        .unregister_all()
        .map_err(|e| e.to_string())?;

    if let Err(e) = app.global_shortcut().register(new_shortcut) {
        // Melhor esforço de rollback pro atalho anterior
        if let Ok(prev) = previous.parse::<Shortcut>() {
            let _ = app.global_shortcut().register(prev);
        }
        return Err(format!(
            "Falha ao registrar '{}': {} (outra aplicação pode estar usando a combinação)",
            trimmed, e
        ));
    }

    settings::save_hotkey(&trimmed).map_err(|e| e.to_string())?;
    tracing::info!("hotkey global atualizada para '{}'", trimmed);
    Ok(())
}

/// Abre um arquivo ou pasta no aplicativo padrão do sistema (browser para HTML,
/// Explorer para pastas). Windows-only — usa `cmd /c start`. Evita adicionar
/// `tauri-plugin-shell` ou `tauri-plugin-opener` só para isso.
#[tauri::command]
pub fn open_in_system(path: String) -> Result<(), String> {
    use std::process::Command;
    Command::new("cmd")
        .args(["/c", "start", "", &path])
        .spawn()
        .map_err(|e| format!("falha ao abrir {}: {}", path, e))?;
    Ok(())
}

/// Pede à IA pra sugerir cenários/regressão/riscos a partir do input do FormView.
/// Frontend pode pré-preencher os textareas do TestesView com o retorno.
#[tauri::command]
pub async fn suggest_test_scenarios(
    form_input: String,
) -> Result<deepseek::TestScenariosSuggestion, String> {
    let api_key = settings::load_api_key()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "API key da DeepSeek não configurada.".to_string())?;

    deepseek::suggest_test_scenarios(&api_key, &form_input)
        .await
        .map_err(|e| format!("Sugestão falhou: {}", e))
}

// ───────────────────────────────────────────────────────────────────────────────
// Dashboard de estatísticas — mede a meta "edições tendendo a zero" a partir do
// histórico SQLite. Parsing puro, sem IA.
// ───────────────────────────────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct WeekStat {
    /// Chave ordenável "AAAA-Wss" (semana ISO), ex: "2026-W28"
    pub week: String,
    pub total: usize,
    pub edited: usize,
}

#[derive(Serialize)]
pub struct CategoryStat {
    pub category: String,
    pub total: usize,
    pub edited: usize,
}

#[derive(Serialize)]
pub struct LearningStats {
    pub total: usize,
    pub approved: usize,
    pub discarded: usize,
    pub edited_approved: usize,
    /// Últimas 12 semanas ISO com aprovadas (semanas sem atividade não aparecem)
    pub weeks: Vec<WeekStat>,
    /// Aprovadas por categoria, mais frequente primeiro
    pub categories: Vec<CategoryStat>,
}

/// Agrega o histórico em métricas de evolução: taxa de edição por semana ISO e
/// volume por categoria (só aprovadas — descartadas entram apenas no total).
#[tauri::command]
pub fn learning_stats(history_state: State<'_, HistoryState>) -> Result<LearningStats, String> {
    let meta = history_state
        .history
        .list_stats_meta()
        .map_err(|e| e.to_string())?;

    let total = meta.len();
    let approved_meta: Vec<_> = meta.iter().filter(|(_, approved, _, _)| *approved).collect();
    let approved = approved_meta.len();
    let discarded = total - approved;
    let edited_approved = approved_meta.iter().filter(|(_, _, edited, _)| *edited).count();

    // Buckets semanais (semana ISO) — BTreeMap mantém ordem cronológica
    let mut weeks: std::collections::BTreeMap<String, (usize, usize)> =
        std::collections::BTreeMap::new();
    for (created_at, _, edited, _) in &approved_meta {
        let Some(dt) = chrono::DateTime::from_timestamp(*created_at, 0) else {
            continue;
        };
        use chrono::Datelike;
        let iso = dt.iso_week();
        let key = format!("{}-W{:02}", iso.year(), iso.week());
        let slot = weeks.entry(key).or_insert((0, 0));
        slot.0 += 1;
        if *edited {
            slot.1 += 1;
        }
    }
    let weeks: Vec<WeekStat> = weeks
        .into_iter()
        .map(|(week, (total, edited))| WeekStat { week, total, edited })
        .collect();
    let weeks = if weeks.len() > 12 {
        weeks[weeks.len() - 12..].to_vec()
    } else {
        weeks
    };

    // Por categoria (aprovadas)
    let mut cats: std::collections::HashMap<String, (usize, usize)> =
        std::collections::HashMap::new();
    for (_, _, edited, category) in &approved_meta {
        let key = category.clone().unwrap_or_else(|| "sem-categoria".to_string());
        let slot = cats.entry(key).or_insert((0, 0));
        slot.0 += 1;
        if *edited {
            slot.1 += 1;
        }
    }
    let mut categories: Vec<CategoryStat> = cats
        .into_iter()
        .map(|(category, (total, edited))| CategoryStat { category, total, edited })
        .collect();
    categories.sort_by(|a, b| b.total.cmp(&a.total).then(a.category.cmp(&b.category)));

    Ok(LearningStats {
        total,
        approved,
        discarded,
        edited_approved,
        weeks,
        categories,
    })
}

/// Extrai os campos do FormView a partir do texto bruto de um ticket (conversa,
/// descrição ou anotações). Frontend mescla o retorno com o formulário atual,
/// preservando o que o usuário já digitou.
#[tauri::command]
pub async fn extract_form_fields(
    ticket_text: String,
) -> Result<deepseek::FormFieldsSuggestion, String> {
    let api_key = settings::load_api_key()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "API key da DeepSeek não configurada.".to_string())?;

    deepseek::extract_form_fields(&api_key, &ticket_text)
        .await
        .map_err(|e| format!("Extração falhou: {}", e))
}

// ───────────────────────────────────────────────────────────────────────────────
// Frases-modelo via IA (#21) — extrai templates recorrentes do final_output
// das aprovadas e propõe acréscimos à seção "Frases-modelo aprovadas" do
// campos-padrao.md.
// ───────────────────────────────────────────────────────────────────────────────

const ANALYZE_PHRASES_LIMIT: usize = 80;
const ANALYZE_PHRASES_MIN: usize = 5;

/// Lê até 80 aprovadas + o campos-padrao.md atual e pede à IA frases-modelo
/// recorrentes que ainda não estão no arquivo. NÃO escreve em lugar nenhum.
#[tauri::command]
pub async fn analyze_phrase_templates(
    history_state: State<'_, HistoryState>,
    vault_state: State<'_, VaultState>,
) -> Result<Vec<deepseek::PhraseTemplate>, String> {
    let api_key = settings::load_api_key()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "API key da DeepSeek não configurada.".to_string())?;

    let vault_path = vault_state
        .loader
        .read()
        .unwrap()
        .status()
        .path
        .clone()
        .ok_or_else(|| "Vault não configurado — selecione a pasta nas Configurações.".to_string())?;

    let entries = history_state
        .history
        .list_recent(ANALYZE_PHRASES_LIMIT, true)
        .map_err(|e| e.to_string())?;

    if entries.len() < ANALYZE_PHRASES_MIN {
        return Err(format!(
            "Apenas {} aprovada(s) disponível(eis). Aprove ao menos {} devolutivas para gerar sugestões.",
            entries.len(),
            ANALYZE_PHRASES_MIN
        ));
    }

    let samples: Vec<String> = entries.into_iter().map(|e| e.final_output).collect();
    let current_campos = vault::read_campos_padrao(&PathBuf::from(&vault_path));

    tracing::info!(
        "analyze_phrase_templates: {} amostras, campos atual: {} chars",
        samples.len(),
        current_campos.len()
    );

    let templates = deepseek::extract_phrase_templates(&api_key, &samples, &current_campos)
        .await
        .map_err(|e| format!("Análise falhou: {}", e))?;

    tracing::info!("DeepSeek retornou {} templates", templates.len());
    Ok(templates)
}

/// Appenda os templates aceitos à seção "Frases-modelo aprovadas" do
/// campos-padrao.md. Backup em .bak + reload do vault + emit `vault-changed`.
#[tauri::command]
pub fn apply_phrase_templates(
    templates: Vec<deepseek::PhraseTemplate>,
    app: AppHandle,
    vault_state: State<'_, VaultState>,
) -> Result<String, String> {
    if templates.is_empty() {
        return Err("Nenhuma frase-modelo selecionada.".to_string());
    }

    let vault_path = vault_state
        .loader
        .read()
        .unwrap()
        .status()
        .path
        .clone()
        .ok_or_else(|| "Vault não configurado — selecione a pasta nas Configurações.".to_string())?;

    let pairs: Vec<(String, String)> = templates
        .into_iter()
        .map(|t| (t.situation, t.template))
        .collect();

    let path = PathBuf::from(&vault_path);
    let written = vault::append_phrase_templates(&path, &pairs).map_err(|e| e.to_string())?;

    {
        let mut l = vault_state.loader.write().unwrap();
        l.reload();
        let _ = app.emit("vault-changed", l.status().clone());
    }
    maybe_git_backup(&path);

    Ok(written
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "campos-padrao.md".to_string()))
}

// ── Backup git do vault (#10) ───────────────────────────────────────────────

/// Dispara o backup git do vault se o toggle estiver ligado nas Configurações.
/// Best-effort: falha vira warn no log — nunca quebra a operação que acabou de
/// escrever no vault.
fn maybe_git_backup(vault_path: &std::path::Path) {
    if !settings::load_vault_git_backup() {
        return;
    }
    match vault::git_backup(vault_path) {
        Ok(true) => tracing::info!("backup git do vault commitado"),
        Ok(false) => {}
        Err(e) => tracing::warn!("backup git do vault falhou: {}", e),
    }
}

#[tauri::command]
pub fn get_vault_git_backup() -> Result<bool, String> {
    Ok(settings::load_vault_git_backup())
}

/// Liga/desliga o backup git automático. Ao ligar com vault configurado, roda
/// um backup inicial imediato — se o git não estiver disponível, o erro volta
/// pro usuário e o toggle não é persistido.
#[tauri::command]
pub fn set_vault_git_backup(
    enabled: bool,
    vault_state: State<'_, VaultState>,
) -> Result<(), String> {
    if enabled {
        let vault_path = vault_state.loader.read().unwrap().status().path.clone();
        if let Some(p) = vault_path {
            vault::git_backup(&PathBuf::from(&p)).map_err(|e| e.to_string())?;
        }
    }
    settings::save_vault_git_backup(enabled).map_err(|e| e.to_string())
}

// ── Autostart (Fase 4) ─────────────────────────────────────────────────────

#[tauri::command]
pub fn get_autostart_enabled(app: AppHandle) -> Result<bool, String> {
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_autostart_enabled(enabled: bool, app: AppHandle) -> Result<(), String> {
    let mgr = app.autolaunch();
    if enabled {
        mgr.enable().map_err(|e| e.to_string())
    } else {
        mgr.disable().map_err(|e| e.to_string())
    }
}

// ── Updater (Fase 4) ───────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct UpdateInfo {
    pub available: bool,
    pub current_version: String,
    pub new_version: Option<String>,
    pub release_notes: Option<String>,
}

/// Consulta o endpoint do updater (GitHub releases) e retorna info de atualização.
/// NÃO baixa nada — só checa. Download/install via `download_and_install_update`.
#[tauri::command]
pub async fn check_for_update(app: AppHandle) -> Result<UpdateInfo, String> {
    let current_version = app.package_info().version.to_string();
    let updater = app.updater().map_err(|e| e.to_string())?;
    match updater.check().await {
        Ok(Some(update)) => Ok(UpdateInfo {
            available: true,
            current_version,
            new_version: Some(update.version.clone()),
            release_notes: update.body.clone(),
        }),
        Ok(None) => Ok(UpdateInfo {
            available: false,
            current_version,
            new_version: None,
            release_notes: None,
        }),
        Err(e) => Err(format!("Checagem falhou: {}", e)),
    }
}

/// Baixa e instala a nova versão. App fecha automaticamente após instalar.
#[tauri::command]
pub async fn download_and_install_update(app: AppHandle) -> Result<(), String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    let update = updater
        .check()
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Nenhuma atualização disponível.".to_string())?;

    update
        .download_and_install(|_chunk, _total| {}, || {})
        .await
        .map_err(|e| format!("Instalação falhou: {}", e))?;

    // Após instalar, reinicia o app
    app.restart();
}

#[tauri::command]
pub async fn close_chat(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("chat") {
        window.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn get_api_key() -> Result<Option<String>, String> {
    settings::load_api_key().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_api_key(key: String) -> Result<(), String> {
    settings::save_api_key(&key).map_err(|e| format!("save: {}", e))?;
    match settings::load_api_key() {
        Ok(Some(loaded)) => {
            if loaded == key {
                Ok(())
            } else {
                Err(format!(
                    "Roundtrip incompleto: escreveu {} chars mas leu {} chars",
                    key.len(),
                    loaded.len()
                ))
            }
        }
        Ok(None) => Err(
            "Save aparentou OK mas leitura retornou vazio (config.json indisponível?)".into(),
        ),
        Err(e) => Err(format!("Save OK mas leitura falhou: {}", e)),
    }
}

#[tauri::command]
pub fn get_vault_path() -> Option<String> {
    settings::load_config().vault_path
}

#[tauri::command]
pub fn set_vault_path(
    app: AppHandle,
    path: String,
    state: State<'_, VaultState>,
) -> Result<VaultStatus, String> {
    let mut config = settings::load_config();
    config.vault_path = Some(path.clone());
    settings::save_config(&config).map_err(|e| e.to_string())?;

    let path_buf = PathBuf::from(&path);
    {
        let mut l = state.loader.write().unwrap();
        l.set_path(path_buf.clone());
    }

    let new_watcher = vault::start_watcher(path_buf, state.loader.clone(), app.clone())
        .map_err(|e| e.to_string())?;
    {
        let mut w = state.watcher.lock().unwrap();
        *w = Some(new_watcher);
    }

    let status = state.loader.read().unwrap().status().clone();
    let _ = app.emit("vault-changed", status.clone());
    Ok(status)
}

#[tauri::command]
pub fn get_vault_status(state: State<'_, VaultState>) -> VaultStatus {
    state.loader.read().unwrap().status().clone()
}

#[tauri::command]
pub fn seed_vault(path: String) -> Result<Vec<String>, String> {
    let p = PathBuf::from(&path);
    let files = vault::seed_vault(&p).map_err(|e| e.to_string())?;
    maybe_git_backup(&p);
    Ok(files)
}

#[derive(Serialize, Clone)]
struct TokenEvent {
    content: String,
}

#[derive(Serialize, Clone)]
struct CategoryEvent {
    category: String,
    examples_used: usize,
}

/// Stream completion com:
/// 1. Classificação do input (1 chamada DeepSeek leve)
/// 2. Leitura do arquivo `exemplos-{categoria}.md` no vault (fonte de verdade da injeção)
/// 3. Stream da geração principal com o arquivo inteiro injetado no system prompt
///
/// O arquivo é injetado INTEIRO — qualquer instrução ou nota que o usuário escrever
/// nele (além dos blocos auto-gerados) chega à IA. Isso preserva a autonomia do
/// usuário sobre o comportamento por-categoria.
#[tauri::command]
pub async fn stream_completion(
    app: AppHandle,
    user_input: String,
    vault_state: State<'_, VaultState>,
    history_state: State<'_, HistoryState>,
) -> Result<(), String> {
    let api_key = settings::load_api_key()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "API key da DeepSeek não configurada.".to_string())?;

    // 1. Classifica o input para decidir qual arquivo de exemplos ler.
    let existing_categories = history_state
        .history
        .list_categories()
        .map_err(|e| e.to_string())?;

    let category = match deepseek::classify(&api_key, &user_input, &existing_categories).await {
        Ok(c) => {
            tracing::info!("input categorizado como '{}'", c);
            c
        }
        Err(e) => {
            tracing::warn!("classificação falhou ({:?}) — usando 'geral'", e);
            "geral".to_string()
        }
    };

    // 2. Lê o arquivo exemplos-{categoria}.md do vault (se existir).
    //    A IA verá: blocos `## Aprovado em ...` auto-gerados + qualquer texto manual
    //    que o usuário tenha adicionado (notas, instruções, exemplos próprios).
    let category_examples: Option<String> = {
        let loader = vault_state.loader.read().unwrap();
        let vault_path = loader.status().path.clone();
        vault_path
            .map(std::path::PathBuf::from)
            .and_then(|p| vault::load_category_examples(&p, &category))
    };

    let examples_count = category_examples
        .as_deref()
        .map(vault::count_example_blocks)
        .unwrap_or(0);
    let examples_chars = category_examples.as_deref().map(str::len).unwrap_or(0);

    tracing::info!(
        "categoria '{}' → exemplos-{}.md: {} blocos, {} chars",
        category,
        category,
        examples_count,
        examples_chars
    );

    if let Some(chat) = app.get_webview_window("chat") {
        let _ = chat.emit(
            "category-detected",
            CategoryEvent {
                category: category.clone(),
                examples_used: examples_count,
            },
        );
    }

    // 3. Monta system prompt com vault rules + arquivo da categoria, depois user_input
    let messages: Vec<ChatMessage> = {
        let loader = vault_state.loader.read().unwrap();
        let pb = PromptBuilder::new(loader.context())
            .with_category(&category, category_examples.as_deref());
        pb.build_messages(&user_input)
    };

    // 4. Stream
    let app_emit = app.clone();
    let result = deepseek::stream_chat(&api_key, messages, move |token| {
        if let Some(chat) = app_emit.get_webview_window("chat") {
            let _ = chat.emit(
                "deepseek-token",
                TokenEvent {
                    content: token.to_string(),
                },
            );
        }
    })
    .await;

    if let Some(chat) = app.get_webview_window("chat") {
        let _ = chat.emit("deepseek-done", ());
    }
    result.map_err(|e| e.to_string())
}

// ───────────────────────────────────────────────────────────────────────────────
// Histórico + auto-curadoria
// ───────────────────────────────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct ApprovalResult {
    pub id: i64,
    pub category: String,
    pub examples_file: Option<String>,
}

#[tauri::command]
pub async fn approve_entry(
    raw_input: String,
    ai_raw_output: String,
    final_output: String,
    history_state: State<'_, HistoryState>,
    vault_state: State<'_, VaultState>,
) -> Result<ApprovalResult, String> {
    // Categoriza usando o final_output (o que o usuário aprovou). Fallback gracioso.
    let api_key = settings::load_api_key()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "API key não configurada (classificação requer DeepSeek).".to_string())?;

    let existing_categories = history_state
        .history
        .list_categories()
        .map_err(|e| e.to_string())?;

    let category = match deepseek::classify(&api_key, &raw_input, &existing_categories).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("classify falhou no approve ({:?}) — usando 'geral'", e);
            "geral".to_string()
        }
    };

    let id = history_state
        .history
        .save(
            &raw_input,
            &ai_raw_output,
            &final_output,
            true,
            "deepseek-chat",
            Some(&category),
        )
        .map_err(|e| e.to_string())?;

    // Append em exemplos-{categoria}.md (vault apenas leitura humana).
    let vault_path = vault_state.loader.read().unwrap().status().path.clone();
    let examples_file = if let Some(p) = vault_path {
        let path = PathBuf::from(p);
        let file_written = match vault::append_to_category_examples(&path, &category, &raw_input, &final_output) {
            Ok(file) => {
                tracing::info!(
                    "entry #{} ({}) appendado em {:?}",
                    id,
                    category,
                    file.file_name().unwrap_or_default()
                );
                Some(
                    file.file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                )
            }
            Err(e) => {
                tracing::error!(
                    "entry #{} salvo no SQLite mas append falhou: {}",
                    id,
                    e
                );
                None
            }
        };
        maybe_git_backup(&path);
        file_written
    } else {
        tracing::info!(
            "entry #{} aprovado (vault não configurado, pulando .md per-categoria)",
            id
        );
        None
    };

    Ok(ApprovalResult {
        id,
        category,
        examples_file,
    })
}

#[tauri::command]
pub async fn discard_entry(
    raw_input: String,
    ai_raw_output: String,
    final_output: String,
    history_state: State<'_, HistoryState>,
) -> Result<i64, String> {
    // Classifica também os descartados — info útil para future evitar.md por categoria.
    // Mas sem bloquear: se classify falhar, salva sem categoria.
    let category = match settings::load_api_key() {
        Ok(Some(api_key)) => {
            let existing = history_state
                .history
                .list_categories()
                .unwrap_or_default();
            deepseek::classify(&api_key, &raw_input, &existing).await.ok()
        }
        _ => None,
    };

    let id = history_state
        .history
        .save(
            &raw_input,
            &ai_raw_output,
            &final_output,
            false,
            "deepseek-chat",
            category.as_deref(),
        )
        .map_err(|e| e.to_string())?;
    tracing::info!(
        "entry #{} descartado (categoria={:?}, sinal negativo registrado)",
        id,
        category
    );
    Ok(id)
}

#[tauri::command]
pub fn list_history(
    limit: Option<usize>,
    approved_only: Option<bool>,
    history_state: State<'_, HistoryState>,
) -> Result<Vec<Entry>, String> {
    history_state
        .history
        .list_recent(limit.unwrap_or(50), approved_only.unwrap_or(false))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn search_history(
    query: String,
    limit: Option<usize>,
    history_state: State<'_, HistoryState>,
) -> Result<Vec<Entry>, String> {
    history_state
        .history
        .search(&query, limit.unwrap_or(50))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_history_entry(
    id: i64,
    history_state: State<'_, HistoryState>,
) -> Result<(), String> {
    history_state
        .history
        .delete(id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn history_count(
    approved_only: Option<bool>,
    history_state: State<'_, HistoryState>,
) -> Result<usize, String> {
    history_state
        .history
        .count(approved_only.unwrap_or(false))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_categories(
    history_state: State<'_, HistoryState>,
) -> Result<Vec<String>, String> {
    history_state
        .history
        .list_categories()
        .map_err(|e| e.to_string())
}

// ───────────────────────────────────────────────────────────────────────────────
// Aprendizado: analisar edições e propor adições ao evitar.md
// ───────────────────────────────────────────────────────────────────────────────

const ANALYZE_EDITS_LIMIT: usize = 20;

#[tauri::command]
pub fn count_edited_approved(history_state: State<'_, HistoryState>) -> Result<usize, String> {
    history_state
        .history
        .count_edited_approved()
        .map_err(|e| e.to_string())
}

/// Analisa as últimas N (até 20) edições aprovadas e retorna sugestões de expressões
/// a evitar. NÃO escreve em lugar nenhum — só retorna sugestões para o usuário revisar.
/// O usuário escolhe quais aceitar via `apply_evitar_suggestions`.
#[tauri::command]
pub async fn analyze_edits(
    history_state: State<'_, HistoryState>,
) -> Result<Vec<deepseek::EvitarSuggestion>, String> {
    let api_key = settings::load_api_key()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "API key da DeepSeek não configurada.".to_string())?;

    let entries = history_state
        .history
        .list_edited_approved(ANALYZE_EDITS_LIMIT)
        .map_err(|e| e.to_string())?;

    if entries.len() < 2 {
        return Err(format!(
            "Apenas {} edição(ões) aprovada(s) disponível(eis). Edite e aprove mais devolutivas para gerar sugestões úteis.",
            entries.len()
        ));
    }

    let pairs: Vec<(String, String)> = entries
        .into_iter()
        .map(|e| (e.ai_raw_output, e.final_output))
        .collect();

    tracing::info!("analisando {} pares editados", pairs.len());

    let suggestions = deepseek::analyze_edits(&api_key, &pairs)
        .await
        .map_err(|e| format!("Análise falhou: {}", e))?;

    tracing::info!("DeepSeek retornou {} sugestões", suggestions.len());
    Ok(suggestions)
}

/// Recebe a lista de sugestões aceitas pelo usuário e appenda em `evitar.md`
/// no vault configurado.
#[tauri::command]
pub fn apply_evitar_suggestions(
    suggestions: Vec<deepseek::EvitarSuggestion>,
    vault_state: State<'_, VaultState>,
) -> Result<String, String> {
    if suggestions.is_empty() {
        return Err("Nenhuma sugestão selecionada.".to_string());
    }

    let vault_path = vault_state
        .loader
        .read()
        .unwrap()
        .status()
        .path
        .clone()
        .ok_or_else(|| "Vault não configurado — selecione a pasta nas Configurações.".to_string())?;

    let pairs: Vec<(String, String)> = suggestions
        .into_iter()
        .map(|s| (s.expression, s.reason))
        .collect();

    let path = std::path::PathBuf::from(vault_path);
    let written = vault::append_to_evitar(&path, &pairs).map_err(|e| e.to_string())?;
    maybe_git_backup(&path);

    Ok(written
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "evitar.md".to_string()))
}

// ───────────────────────────────────────────────────────────────────────────────
// Síntese de estilo.md (#19) — loop de aprendizado positivo
// ───────────────────────────────────────────────────────────────────────────────

const SYNTHESIZE_STYLE_LIMIT: usize = 50;
const SYNTHESIZE_STYLE_MIN: usize = 5;

#[tauri::command]
pub fn count_approved_unedited(history_state: State<'_, HistoryState>) -> Result<usize, String> {
    history_state
        .history
        .count_approved_unedited()
        .map_err(|e| e.to_string())
}

/// Sintetiza nova proposta de estilo.md baseada em até 50 aprovadas-sem-edição.
/// NÃO escreve em lugar nenhum — só retorna o markdown da proposta. O usuário
/// revisa (e opcionalmente edita) num textarea antes de aplicar via `apply_style_synthesis`.
#[tauri::command]
pub async fn synthesize_style(
    history_state: State<'_, HistoryState>,
    vault_state: State<'_, VaultState>,
) -> Result<String, String> {
    let api_key = settings::load_api_key()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "API key da DeepSeek não configurada.".to_string())?;

    let vault_path = vault_state
        .loader
        .read()
        .unwrap()
        .status()
        .path
        .clone()
        .ok_or_else(|| "Vault não configurado — selecione a pasta nas Configurações.".to_string())?;

    let current_style = std::fs::read_to_string(PathBuf::from(&vault_path).join("estilo.md"))
        .unwrap_or_default();

    let entries = history_state
        .history
        .list_approved_unedited(SYNTHESIZE_STYLE_LIMIT)
        .map_err(|e| e.to_string())?;

    if entries.len() < SYNTHESIZE_STYLE_MIN {
        return Err(format!(
            "Apenas {} aprovada(s) sem edição disponível(eis). Aprove ao menos {} devolutivas SEM editar para sintetizar.",
            entries.len(),
            SYNTHESIZE_STYLE_MIN
        ));
    }

    let samples: Vec<(String, String)> = entries
        .into_iter()
        .map(|e| (e.raw_input, e.final_output))
        .collect();

    tracing::info!(
        "sintetizando estilo.md a partir de {} aprovadas sem edição (estilo atual: {} chars)",
        samples.len(),
        current_style.len()
    );

    let proposal = deepseek::synthesize_style(&api_key, &current_style, &samples)
        .await
        .map_err(|e| format!("Síntese falhou: {}", e))?;

    tracing::info!("DeepSeek retornou proposta de {} chars", proposal.len());
    Ok(proposal)
}

/// Substitui o estilo.md do vault pelo conteúdo recebido (potencialmente editado
/// pelo usuário no textarea). Faz backup em estilo.md.bak (sobrescrito a cada vez)
/// e recarrega o vault loader. O watcher do filesystem TAMBÉM detecta a escrita e
/// dispara reload, mas reload manual aqui é defesa em profundidade — operação é
/// idempotente, então a duplicação não causa problema.
#[tauri::command]
pub fn apply_style_synthesis(
    new_content: String,
    app: AppHandle,
    vault_state: State<'_, VaultState>,
) -> Result<String, String> {
    let vault_path = vault_state
        .loader
        .read()
        .unwrap()
        .status()
        .path
        .clone()
        .ok_or_else(|| "Vault não configurado — selecione a pasta nas Configurações.".to_string())?;

    let path = PathBuf::from(&vault_path);
    let written = vault::replace_estilo(&path, &new_content).map_err(|e| e.to_string())?;

    {
        let mut l = vault_state.loader.write().unwrap();
        l.reload();
        let _ = app.emit("vault-changed", l.status().clone());
    }
    maybe_git_backup(&path);

    Ok(written
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "estilo.md".to_string()))
}

// ───────────────────────────────────────────────────────────────────────────────
// Stats em campos-padrao.md (#20) — releases + caminhos via parsing puro
// ───────────────────────────────────────────────────────────────────────────────

const ANALYZE_CAMPOS_LIMIT: usize = 100;
const ANALYZE_CAMPOS_MIN: usize = 5;
const PATHS_MIN_OCCURRENCES: u32 = 2;
const PATHS_TOP_N: usize = 10;

/// Analisa o histórico de aprovadas (sem chamar IA) para detectar a release mais
/// recente e os caminhos mais frequentes que ainda NÃO estão no `campos-padrao.md`.
/// Retorna sugestões para o usuário revisar via checkbox.
#[tauri::command]
pub fn analyze_campos(
    history_state: State<'_, HistoryState>,
    vault_state: State<'_, VaultState>,
) -> Result<CamposSuggestions, String> {
    let vault_path = vault_state
        .loader
        .read()
        .unwrap()
        .status()
        .path
        .clone()
        .ok_or_else(|| "Vault não configurado — selecione a pasta nas Configurações.".to_string())?;

    let entries = history_state
        .history
        .list_recent(ANALYZE_CAMPOS_LIMIT, true)
        .map_err(|e| e.to_string())?;

    if entries.len() < ANALYZE_CAMPOS_MIN {
        return Err(format!(
            "Apenas {} aprovada(s) disponível(eis). Aprove ao menos {} devolutivas para gerar sugestões.",
            entries.len(),
            ANALYZE_CAMPOS_MIN
        ));
    }
    let analyzed_count = entries.len();

    let path = PathBuf::from(&vault_path);
    let campos_text = vault::read_campos_padrao(&path);
    let current_release = vault::current_release_in_text(&campos_text);
    let existing_paths = vault::existing_paths_in_text(&campos_text);

    // Coleta releases das entries; pega a maior por semver.
    let mut releases = Vec::new();
    let mut all_paths = Vec::new();
    for e in &entries {
        if let Some((canon, semver, date)) = stats::extract_release(&e.raw_input) {
            releases.push((canon, semver, date));
        }
        for p in stats::extract_paths(&e.raw_input) {
            all_paths.push(p);
        }
    }

    let release_suggestion = stats::pick_latest_release(&releases).and_then(|proposed| {
        if Some(&proposed) == current_release.as_ref() {
            None // já é a versão atual no arquivo
        } else {
            Some(ReleaseSuggestion {
                proposed,
                current_in_file: current_release.clone(),
            })
        }
    });

    let ranked = stats::rank_paths(&all_paths, PATHS_MIN_OCCURRENCES);
    let path_suggestions: Vec<PathSuggestion> = ranked
        .into_iter()
        .filter(|p| !existing_paths.contains(&stats::normalize_path(&p.path)))
        .take(PATHS_TOP_N)
        .collect();

    tracing::info!(
        "analyze_campos: {} entries analisadas; release sugerida={:?}; {} novos caminhos",
        analyzed_count,
        release_suggestion.as_ref().map(|r| &r.proposed),
        path_suggestions.len()
    );

    Ok(CamposSuggestions {
        release: release_suggestion,
        paths: path_suggestions,
        analyzed_count,
    })
}

/// Aplica as sugestões aceitas pelo usuário em `campos-padrao.md`. Faz backup em
/// `campos-padrao.md.bak`, recarrega o vault e emite `vault-changed`.
///
/// - `release_accepted`: se Some, substitui a linha "Release atual" pelo valor (que
///   deve ser a `proposed` string da sugestão).
/// - `paths_accepted`: caminhos a appendar (vindos das sugestões marcadas pelo usuário).
#[tauri::command]
pub fn apply_campos_suggestions(
    release_accepted: Option<String>,
    paths_accepted: Vec<String>,
    app: AppHandle,
    vault_state: State<'_, VaultState>,
) -> Result<String, String> {
    if release_accepted.is_none() && paths_accepted.is_empty() {
        return Err("Nenhuma sugestão selecionada.".to_string());
    }

    let vault_path = vault_state
        .loader
        .read()
        .unwrap()
        .status()
        .path
        .clone()
        .ok_or_else(|| "Vault não configurado — selecione a pasta nas Configurações.".to_string())?;

    let path = PathBuf::from(&vault_path);
    let written = vault::apply_campos_changes(&path, release_accepted.as_deref(), &paths_accepted)
        .map_err(|e| e.to_string())?;

    {
        let mut l = vault_state.loader.write().unwrap();
        l.reload();
        let _ = app.emit("vault-changed", l.status().clone());
    }
    maybe_git_backup(&path);

    Ok(written
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "campos-padrao.md".to_string()))
}
