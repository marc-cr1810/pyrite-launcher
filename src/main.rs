// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod core;

slint::include_modules!();

fn main() -> Result<(), slint::PlatformError> {
    app::run()
}
