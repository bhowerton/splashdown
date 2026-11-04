//! Tauri's build step: generates the app's context (config, icons,
//! capabilities) at compile time.

fn main() {
    tauri_build::build()
}
