mod commands;
mod deepseek;
mod history;
mod prompt;
mod settings;
mod stats;
mod vault;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager,
};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
use tracing_subscriber::EnvFilter;

/// Hotkey hardcoded de 1ª versão. Configurabilidade via Settings é trabalho de
/// iteração futura — exige UI de captura de tecla + persistência + re-registro.
fn default_hotkey() -> Shortcut {
    Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyD)
}

use commands::{HistoryState, VaultState};
use history::History;
use vault::VaultLoader;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,artemis=debug")),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None, // sem args extras
        ))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state() == ShortcutState::Pressed && shortcut == &default_hotkey() {
                        show_chat(app);
                    }
                })
                .build(),
        )
        .setup(|app| {
            // Posiciona o FAB no canto inferior direito do monitor primário
            if let Some(fab) = app.get_webview_window("fab") {
                if let Ok(Some(monitor)) = fab.primary_monitor() {
                    let size = monitor.size();
                    let pos = monitor.position();
                    let scale = monitor.scale_factor();
                    let fab_phys = (72.0 * scale) as i32;
                    let margin_right = (24.0 * scale) as i32;
                    let margin_bottom = (60.0 * scale) as i32;
                    let x = pos.x + size.width as i32 - fab_phys - margin_right;
                    let y = pos.y + size.height as i32 - fab_phys - margin_bottom;
                    let _ = fab.set_position(tauri::PhysicalPosition::new(x, y));
                }
            }

            // Intercepta o close da janela de chat para esconder em vez de destruir
            if let Some(chat) = app.get_webview_window("chat") {
                let chat_clone = chat.clone();
                chat.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = chat_clone.hide();
                    }
                });
            }

            // Inicializa o histórico SQLite em %APPDATA%/Artemis/history.db
            let db_path = settings::config_dir()
                .expect("config_dir indisponível")
                .join("history.db");
            let history = Arc::new(History::open(&db_path).expect("falha ao abrir history.db"));
            tracing::info!("histórico SQLite aberto em {:?}", db_path);

            // Inicializa o VaultLoader e, se já houver vault_path configurado, dispara o watcher
            let loader = Arc::new(RwLock::new(VaultLoader::new()));
            let watcher_holder: Mutex<Option<vault::Watcher>> = Mutex::new(None);

            let config = settings::load_config();
            if let Some(vault_path_str) = config.vault_path {
                let vault_path = PathBuf::from(&vault_path_str);
                if vault_path.exists() {
                    {
                        let mut l = loader.write().unwrap();
                        l.set_path(vault_path.clone());
                    }
                    let app_handle = app.handle().clone();
                    match vault::start_watcher(vault_path, loader.clone(), app_handle) {
                        Ok(w) => {
                            *watcher_holder.lock().unwrap() = Some(w);
                            tracing::info!("vault watcher iniciado");
                        }
                        Err(e) => {
                            tracing::error!("falha ao iniciar vault watcher: {}", e);
                        }
                    }
                } else {
                    tracing::warn!("vault_path configurado não existe: {}", vault_path_str);
                }
            }

            app.manage(VaultState {
                loader,
                watcher: watcher_holder,
            });
            app.manage(HistoryState { history });

            // ── Tray icon + menu (Fase 4) ────────────────────────────────────
            // Click esquerdo no ícone → abre o chat.
            // Menu (click direito): Abrir chat / Configurações / Sair.
            // "Configurações" emite event `tray-open-settings` que o ChatWindow escuta
            // para entrar direto na SettingsPanel.
            let open_item = MenuItem::with_id(app, "tray_open", "Abrir chat", true, None::<&str>)?;
            let settings_item = MenuItem::with_id(app, "tray_settings", "Configurações", true, None::<&str>)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let quit_item = MenuItem::with_id(app, "tray_quit", "Sair do Artemis", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open_item, &settings_item, &sep, &quit_item])?;

            let _tray = TrayIconBuilder::with_id("artemis-tray")
                .icon(app.default_window_icon().expect("default window icon ausente").clone())
                .tooltip("Artemis — devolutivas técnicas")
                .menu(&menu)
                .show_menu_on_left_click(false) // click esquerdo abre o chat, não o menu
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "tray_open" => {
                        show_chat(app);
                    }
                    "tray_settings" => {
                        show_chat(app);
                        let _ = app.emit("tray-open-settings", ());
                    }
                    "tray_quit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_chat(tray.app_handle());
                    }
                })
                .build(app)?;

            // ── Global shortcut: Ctrl+Shift+D abre o chat ───────────────────
            // O handler foi registrado no Builder; aqui só ativamos a captura.
            if let Err(e) = app.global_shortcut().register(default_hotkey()) {
                tracing::warn!("falha ao registrar hotkey global Ctrl+Shift+D: {} (outra app pode estar usando)", e);
            } else {
                tracing::info!("hotkey global Ctrl+Shift+D ativada");
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::open_chat,
            commands::close_chat,
            commands::get_api_key,
            commands::set_api_key,
            commands::get_vault_path,
            commands::set_vault_path,
            commands::get_vault_status,
            commands::seed_vault,
            commands::stream_completion,
            commands::approve_entry,
            commands::discard_entry,
            commands::list_history,
            commands::search_history,
            commands::delete_history_entry,
            commands::history_count,
            commands::list_categories,
            commands::count_edited_approved,
            commands::analyze_edits,
            commands::apply_evitar_suggestions,
            commands::count_approved_unedited,
            commands::synthesize_style,
            commands::apply_style_synthesis,
            commands::analyze_campos,
            commands::apply_campos_suggestions,
            commands::get_autostart_enabled,
            commands::set_autostart_enabled,
            commands::check_for_update,
            commands::download_and_install_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Mostra e foca a janela de chat. Usada pelo tray (click esquerdo + menu)
/// e pelo handler de hotkey global. Erros são ignorados (best-effort UX).
fn show_chat(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("chat") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
