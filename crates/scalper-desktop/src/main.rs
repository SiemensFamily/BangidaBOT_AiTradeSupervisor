mod app;
mod tabs;
mod ws_client;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_title("Crypto Scalper Desktop"),
        ..Default::default()
    };

    eframe::run_native(
        "Crypto Scalper Desktop",
        options,
        Box::new(|cc| {
            // Force dark theme
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(app::ScalperApp::new(cc)))
        }),
    )
}
