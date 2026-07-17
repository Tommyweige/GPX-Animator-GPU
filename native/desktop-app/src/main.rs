#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> eframe::Result {
    let startup_gpx = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("GPX Animator GPU Edition")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([1100.0, 700.0]),
        ..Default::default()
    };
    eframe::run_native(
        "GPX Animator GPU Edition",
        options,
        Box::new(move |cc| {
            Ok(Box::new(desktop_app::ui::NativeApp::new_with_path(
                cc,
                startup_gpx.clone(),
            )))
        }),
    )
}
