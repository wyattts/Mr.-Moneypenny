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

        // Bring up logging once; cheap and survives reload of tauri::Builder.
        let _ = tracing_subscriber::fmt::try_init();

        let path = db::default_db_path().expect("could not resolve db path");
        let conn = db::open(&path).expect("could not open db");
        db::migrate(&conn).expect("could not migrate db");
        let state = AppState::new(conn);

        tauri::Builder::default()
            .manage(state)
            .setup(|app| {
                use tauri::Manager;
                // Tokio runtime is live by the time setup() runs, so this
                // is the right place to spawn the poll task.
                let state = app.state::<AppState>();
                let setup_complete = {
                    let conn = state.db.lock().unwrap();
                    crate::repository::settings::get(
                        &conn,
                        crate::repository::settings::keys::SETUP_COMPLETE,
                    )
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
                save_currency_locale,
                list_categories,
                set_category_target,
                set_category_active,
                finalize_setup,
            ])
            .run(tauri::generate_context!())
            .expect("error while running tauri application");
    }
}

#[cfg(feature = "desktop")]
pub use app::run;
