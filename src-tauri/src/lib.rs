#![forbid(unsafe_code)]

pub mod db;
pub mod domain;
pub mod insights;
pub mod llm;
pub mod repository;

#[cfg(feature = "desktop")]
mod app {
    #[tauri::command]
    pub(super) fn ping() -> &'static str {
        "pong"
    }

    #[cfg_attr(mobile, tauri::mobile_entry_point)]
    pub fn run() {
        tauri::Builder::default()
            .invoke_handler(tauri::generate_handler![ping])
            .run(tauri::generate_context!())
            .expect("error while running tauri application");
    }
}

#[cfg(feature = "desktop")]
pub use app::run;
