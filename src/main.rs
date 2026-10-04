pub mod app;
pub mod models;
pub mod roku;
pub mod theme;

use app::RokuRemoteApp;
use eframe::egui;

fn main() -> Result<(), eframe::Error> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([420.0, 860.0])
            .with_min_inner_size([320.0, 680.0])
            .with_title("Roku Remote")
            .with_app_id("org.omarchy.roku.remote"),
        ..Default::default()
    };

    eframe::run_native(
        "Roku Remote",
        native_options,
        Box::new(|cc| Ok(Box::new(RokuRemoteApp::new(cc)))),
    )
}
