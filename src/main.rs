#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

pub mod app;
pub mod models;
pub mod roku;
pub mod theme;

use app::RokuRemoteApp;
use eframe::egui;

fn load_icon() -> Option<egui::IconData> {
    let img = image::load_from_memory(include_bytes!("../assets/icon.png")).ok()?.into_rgba8();
    let (width, height) = img.dimensions();
    Some(egui::IconData {
        rgba: img.into_raw(),
        width,
        height,
    })
}

fn main() -> Result<(), eframe::Error> {
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([420.0, 860.0])
        .with_min_inner_size([320.0, 680.0])
        .with_title("Roku Remote")
        .with_app_id("org.omarchy.roku.remote");

    if let Some(icon) = load_icon() {
        viewport = viewport.with_icon(icon);
    }

    let native_options = eframe::NativeOptions {
        viewport,
        persist_window: true,
        ..Default::default()
    };

    eframe::run_native(
        "Roku Remote",
        native_options,
        Box::new(|cc| Ok(Box::new(RokuRemoteApp::new(cc)))),
    )
}
