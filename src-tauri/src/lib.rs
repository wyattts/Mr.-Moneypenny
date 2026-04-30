#![forbid(unsafe_code)]

pub mod app_state;
pub mod db;
pub mod domain;
pub mod insights;
pub mod llm;
pub mod repository;
pub mod secrets;
pub mod telegram;

#[cfg(feature = "desktop")]
pub mod commands;

#[cfg(feature = "desktop")]
mod app {
    use crate::app_state::AppState;
    use crate::commands::*;
    use crate::db;
    use crate::repository::settings;

    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    use tauri::{Manager, WindowEvent};
    use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

    #[cfg_attr(mobile, tauri::mobile_entry_point)]
    pub fn run() {
        // WebKitGTK 2.46+ defaults to DMABUF rendering which Error-71's on
        // some Wayland compositors (notably Mutter on Fedora). Force the
        // workaround unless the user has explicitly opted in. This MUST
        // happen before any threads are spawned, so before tracing init.
        #[cfg(target_os = "linux")]
        {
            if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
                std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
            }
            if std::env::var_os("WEBKIT_DISABLE_COMPOSITING_MODE").is_none() {
                std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
            }
        }

        let _ = tracing_subscriber::fmt::try_init();

        let path = db::default_db_path().expect("could not resolve db path");
        let conn = db::open(&path).expect("could not open db");
        db::migrate(&conn).expect("could not migrate db");
        let state = AppState::new(conn);

        let silent = std::env::args().any(|a| a == "--silent");

        tauri::Builder::default()
            .plugin(tauri_plugin_autostart::init(
                MacosLauncher::LaunchAgent,
                Some(vec!["--silent"]),
            ))
            .plugin(tauri_plugin_updater::Builder::new().build())
            .manage(state)
            .setup(move |app| {
                // Apply first-run defaults for bg-mode and autostart, then
                // sync the OS autostart state with our saved setting.
                apply_initial_defaults(app)?;

                // Build tray icon + menu.
                build_tray(app)?;

                // If launched via --silent (e.g., autostart), hide the
                // window so we live entirely in the tray.
                if silent {
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.hide();
                    }
                }

                // Tokio runtime is live by setup() so this is the right
                // place to spawn the poller.
                let state = app.state::<AppState>();
                let setup_complete = {
                    let conn = state.db.lock().unwrap();
                    settings::get(&conn, settings::keys::SETUP_COMPLETE)
                        .ok()
                        .flatten()
                        .as_deref()
                        == Some("1")
                };
                if setup_complete {
                    if let Err(e) = state.ensure_poller_running() {
                        tracing::warn!(target: "app::run", error=%e, "poller did not start at launch");
                    }
                }
                Ok(())
            })
            .on_window_event(|window, event| {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    let state: tauri::State<AppState> = window.state();
                    let bg_mode = {
                        let conn = state.db.lock().unwrap();
                        settings::get(&conn, settings::keys::RUN_IN_BACKGROUND)
                            .ok()
                            .flatten()
                            .as_deref()
                            != Some("0")
                    };
                    if bg_mode {
                        let _ = window.hide();
                        api.prevent_close();
                    }
                }
            })
            .invoke_handler(tauri::generate_handler![
                ping,
                get_setup_state,
                set_setup_step,
                save_llm_provider,
                save_anthropic_key,
                test_anthropic,
                save_ollama_config,
                list_ollama_models,
                test_ollama,
                save_telegram_token,
                generate_pairing_code,
                list_authorized_chats,
                clear_authorized_chats,
                save_currency_locale,
                list_categories,
                set_category_target,
                set_category_active,
                finalize_setup,
                get_dashboard,
                list_expenses,
                delete_expense,
                create_category,
                delete_category,
                remove_household_member,
                get_run_in_background,
                set_run_in_background,
                get_autostart,
                set_autostart,
                check_for_update,
                install_update,
                get_check_updates_on_launch,
                set_check_updates_on_launch,
            ])
            .run(tauri::generate_context!())
            .expect("error while running tauri application");
    }

    /// On first run, apply OS-specific defaults for bg-mode and autostart.
    /// Then sync the OS autostart state to the saved setting (so manual
    /// edits via Preferences/Task Scheduler are corrected on next launch).
    fn apply_initial_defaults(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
        let state = app.state::<AppState>();

        // RUN_IN_BACKGROUND default = 1 on every platform.
        let bg_saved = {
            let conn = state.db.lock().unwrap();
            settings::get(&conn, settings::keys::RUN_IN_BACKGROUND)
                .ok()
                .flatten()
        };
        if bg_saved.is_none() {
            let conn = state.db.lock().unwrap();
            let _ = settings::set(&conn, settings::keys::RUN_IN_BACKGROUND, "1");
        }

        // AUTOSTART: default ON on macOS / Windows, OFF on Linux (because
        // GNOME doesn't show tray icons without the AppIndicator extension).
        let auto_saved = {
            let conn = state.db.lock().unwrap();
            settings::get(&conn, settings::keys::AUTOSTART)
                .ok()
                .flatten()
        };
        let auto_default = cfg!(any(target_os = "macos", target_os = "windows"));
        let auto_desired = match auto_saved.as_deref() {
            Some("1") => true,
            Some("0") => false,
            _ => {
                let conn = state.db.lock().unwrap();
                let _ = settings::set(
                    &conn,
                    settings::keys::AUTOSTART,
                    if auto_default { "1" } else { "0" },
                );
                auto_default
            }
        };
        let manager = app.autolaunch();
        let current = manager.is_enabled().unwrap_or(false);
        if current != auto_desired {
            if auto_desired {
                let _ = manager.enable();
            } else {
                let _ = manager.disable();
            }
        }
        Ok(())
    }

    fn build_tray(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
        let show = MenuItem::with_id(app, "show", "Open Mr. Moneypenny", true, None::<&str>)?;
        let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
        let menu = Menu::with_items(app, &[&show, &quit])?;

        // Tray-specific icon: tighter crop than the bundle/window icon so
        // the character reads at 22-32 px without padding eating the body.
        // Embed at compile time so we don't need a runtime resource lookup.
        let tray_icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray-icon.png"))
            .ok()
            .or_else(|| app.default_window_icon().cloned());

        let mut builder = TrayIconBuilder::with_id("main")
            .menu(&menu)
            .show_menu_on_left_click(false);
        if let Some(i) = tray_icon {
            builder = builder.icon(i);
        }

        let _tray = builder
            .on_tray_icon_event(|tray, event| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    let app = tray.app_handle();
                    show_main_window(app);
                }
            })
            .on_menu_event(|app, event| match event.id.as_ref() {
                "show" => show_main_window(app),
                "quit" => app.exit(0),
                _ => {}
            })
            .build(app)?;

        Ok(())
    }

    fn show_main_window(app: &tauri::AppHandle) {
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.show();
            let _ = w.unminimize();
            let _ = w.set_focus();
        }
    }
}

#[cfg(feature = "desktop")]
pub use app::run;
