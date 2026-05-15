// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Wyatt Smith and contributors
#![forbid(unsafe_code)]

pub mod app_state;
pub mod csv_import;
pub mod db;
pub mod domain;
pub mod insights;
pub mod llm;
pub mod repository;
pub mod scheduler;
pub mod secrets;
pub mod telegram;

#[cfg(feature = "desktop")]
pub mod commands;

#[cfg(feature = "desktop")]
mod app {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use crate::app_state::AppState;
    use crate::commands::*;
    use crate::db;
    use crate::repository::settings;

    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    use tauri::{Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};
    use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

    /// Wraps an AtomicBool stashed in Tauri's managed state so the
    /// Quit menu handler can signal "yes, the user actually wants to
    /// exit" and the run-loop's `ExitRequested` handler can tell that
    /// apart from "the user just closed the window" (which we keep the
    /// process alive through — see v0.3.17 webview-on-close work).
    struct UserQuit(Arc<AtomicBool>);

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
        // Spawn the single-writer DB actor. `spawn_from_path` opens
        // the SQLite file, runs forward migrations, and hands the
        // connection off to a dedicated OS thread. All app code from
        // here on accesses the DB only through the actor (audit
        // CC-1/Pf-4 phases 1–4).
        let db_actor = db::DbHandle::spawn_from_path(&path).expect("could not start db actor");
        let state = AppState::new(db_actor);

        let silent = std::env::args().any(|a| a == "--silent");
        let user_quit = Arc::new(AtomicBool::new(false));

        let app = tauri::Builder::default()
            // Single-instance must be the FIRST plugin: when a second
            // launch is attempted, the plugin shorts the new process's
            // startup, hands its argv to the already-running instance,
            // and exits. Without this, every desktop-icon click on
            // Linux / Windows spawns another full app + tray entry,
            // racking up memory for users who don't realize the tray
            // already has one.
            .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
                // Existing instance hears about the new launch — show
                // (or rebuild, if it was closed-to-tray) and focus the
                // window instead of letting a new one start.
                show_or_create_main_window(app);
            }))
            .plugin(tauri_plugin_autostart::init(
                MacosLauncher::LaunchAgent,
                Some(vec!["--silent"]),
            ))
            .plugin(tauri_plugin_updater::Builder::new().build())
            .plugin(tauri_plugin_dialog::init())
            .manage(state)
            .manage(UserQuit(user_quit.clone()))
            .setup(move |app| {
                // Apply first-run defaults for bg-mode and autostart, then
                // sync the OS autostart state with our saved setting.
                apply_initial_defaults(app)?;

                // Build tray icon + menu.
                build_tray(app)?;

                // Non-silent launch (user double-clicked the icon): build
                // the main window now. Silent launch (autostart at login):
                // skip — we live in the tray only until the user clicks
                // it. tauri.conf.json declares no windows so nothing is
                // auto-created either way. (v0.3.17: avoids loading the
                // WebKit process at all in tray-only mode, which is the
                // ~440 MB resident savings we're after.)
                if !silent {
                    show_or_create_main_window(app.handle());
                }

                // Tokio runtime is live by setup() so this is the right
                // place to spawn the poller.
                let state = app.state::<AppState>();
                let setup_complete = state
                    .db_actor
                    .blocking_run(|conn| {
                        Ok(settings::get(conn, settings::keys::SETUP_COMPLETE)?.as_deref()
                            == Some("1"))
                    })
                    .unwrap_or(false);
                if setup_complete {
                    if let Err(e) = state.ensure_poller_running() {
                        tracing::warn!(target: "app::run", error=%e, "poller did not start at launch");
                    }
                    if let Err(e) = state.ensure_scheduler_running() {
                        tracing::warn!(target: "app::run", error=%e, "scheduler did not start at launch");
                    }
                }
                Ok(())
            })
            // No on_window_event handler: a close request is allowed
            // to actually close the window so the WebKit process exits.
            // The app stays alive via the tray icon + the
            // ExitRequested guard in the run callback below.
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
                get_ollama_allow_remote,
                set_ollama_allow_remote,
                get_llm_daily_cost_cap_micros,
                set_llm_daily_cost_cap_micros,
                report_estimate,
                report_generate,
                report_save_pdf,
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
                get_app_version,
                get_check_updates_on_launch,
                set_check_updates_on_launch,
                get_weekly_summary_enabled,
                set_weekly_summary_enabled,
                get_budget_alerts_enabled,
                set_budget_alerts_enabled,
                get_llm_usage_summary,
                get_category_stats,
                run_scenario,
                set_starting_balance,
                list_investment_categories,
                simulator_solve_required_contribution,
                simulator_compute_probability,
                simulator_heatmap,
                analyze_category,
                debt_simulate_schedule,
                debt_goal_seek,
                debt_simulate_portfolio,
                csv_import_preview,
                csv_import_save_profile,
                csv_import_parse,
                csv_import_categorize_and_dedupe,
                csv_import_ai_suggest,
                csv_import_commit,
                list_csv_import_profiles,
                delete_csv_import_profile,
                list_merchant_rules,
                delete_merchant_rule,
            ])
            .build(tauri::generate_context!())
            .expect("error while building tauri application");

        // Drive the event loop manually so we can intercept
        // ExitRequested. With no startup window declared in
        // tauri.conf.json, closing the last window would otherwise let
        // the event loop wind down and kill the tray + scheduler +
        // poller.
        //
        // - Quit menu / `app.exit()`: UserQuit flag is set first, so
        //   we let exit happen.
        // - User closes the window with RUN_IN_BACKGROUND=1 (default):
        //   prevent exit; tray + scheduler + poller keep running.
        // - User closes the window with RUN_IN_BACKGROUND=0: let exit
        //   happen, same as if Tauri's default behavior were in
        //   effect.
        app.run(move |app_handle, event| {
            if let RunEvent::ExitRequested { api, .. } = event {
                let wants_quit = app_handle.state::<UserQuit>().0.load(Ordering::SeqCst);
                if wants_quit {
                    return;
                }
                let state = app_handle.state::<AppState>();
                let bg_mode = state
                    .db_actor
                    .blocking_run(|conn| {
                        Ok(
                            settings::get(conn, settings::keys::RUN_IN_BACKGROUND)?.as_deref()
                                != Some("0"),
                        )
                    })
                    .unwrap_or(true);
                if bg_mode {
                    api.prevent_exit();
                }
            }
        });
    }

    /// On first run, apply OS-specific defaults for bg-mode and autostart.
    /// Then sync the OS autostart state to the saved setting (so manual
    /// edits via Preferences/Task Scheduler are corrected on next launch).
    fn apply_initial_defaults(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
        let state = app.state::<AppState>();

        // First-run defaults: RUN_IN_BACKGROUND=1 everywhere; AUTOSTART
        // defaults ON on macOS/Windows and OFF on Linux (GNOME doesn't
        // show tray icons without the AppIndicator extension). One
        // actor round-trip reads + writes both keys to keep them
        // consistent across crashes mid-init.
        let auto_default = cfg!(any(target_os = "macos", target_os = "windows"));
        let auto_desired = state.db_actor.blocking_run(move |conn| {
            let bg_saved = settings::get(conn, settings::keys::RUN_IN_BACKGROUND)?;
            if bg_saved.is_none() {
                let _ = settings::set(conn, settings::keys::RUN_IN_BACKGROUND, "1");
            }
            let auto_saved = settings::get(conn, settings::keys::AUTOSTART)?;
            Ok(match auto_saved.as_deref() {
                Some("1") => true,
                Some("0") => false,
                _ => {
                    let _ = settings::set(
                        conn,
                        settings::keys::AUTOSTART,
                        if auto_default { "1" } else { "0" },
                    );
                    auto_default
                }
            })
        })?;
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
                    show_or_create_main_window(tray.app_handle());
                }
            })
            .on_menu_event(|app, event| match event.id.as_ref() {
                "show" => show_or_create_main_window(app),
                "quit" => {
                    // Flip the user-quit flag BEFORE exit() so the
                    // ExitRequested guard lets the process die.
                    app.state::<UserQuit>().0.store(true, Ordering::SeqCst);
                    app.exit(0);
                }
                _ => {}
            })
            .build(app)?;

        Ok(())
    }

    /// Show the main window if it exists, otherwise build it. Building
    /// reloads the bundled HTML/JS and spins up a fresh WebKit process.
    fn show_or_create_main_window(app: &tauri::AppHandle) {
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.show();
            let _ = w.unminimize();
            let _ = w.set_focus();
            return;
        }
        let result = WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
            .title("Mr. Moneypenny")
            .inner_size(1100.0, 740.0)
            .min_inner_size(900.0, 600.0)
            .resizable(true)
            .decorations(true)
            .theme(Some(tauri::Theme::Dark))
            .build();
        if let Err(e) = result {
            tracing::error!(target: "app::run", error=%e, "could not build main window");
        }
    }
}

#[cfg(feature = "desktop")]
pub use app::run;
