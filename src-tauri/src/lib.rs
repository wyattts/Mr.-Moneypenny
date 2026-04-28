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
        // Bring up logging once; cheap and survives reload of tauri::Builder.
        let _ = tracing_subscriber::fmt::try_init();

        let path = db::default_db_path().expect("could not resolve db path");
        let conn = db::open(&path).expect("could not open db");
        db::migrate(&conn).expect("could not migrate db");
        let state = AppState::new(conn);
        // If setup was previously completed, start polling immediately.
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

        tauri::Builder::default()
            .manage(state)
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
