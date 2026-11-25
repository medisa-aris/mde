mod app;
mod document;
mod editor;
mod preview;
mod theme;
mod worker;

#[cfg(test)]
mod perf;

use std::path::PathBuf;
use std::time::Instant;

fn main() -> eframe::Result {
    let started = Instant::now();
    // `-psn_…` is added by Finder when launching an app bundle.
    let files: Vec<PathBuf> = std::env::args_os()
        .skip(1)
        .filter(|a| !a.to_string_lossy().starts_with("-psn_"))
        .map(PathBuf::from)
        .collect();

    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("mde")
            .with_inner_size([1100.0, 750.0])
            .with_min_inner_size([480.0, 320.0]),
        ..Default::default()
    };

    // winit's default macOS menu has a Quit item that ends the process at once,
    // skipping the unsaved-changes prompt. Without it, Cmd+Q reaches the app.
    #[cfg(target_os = "macos")]
    {
        options.event_loop_builder = Some(Box::new(|builder| {
            use winit::platform::macos::EventLoopBuilderExtMacOS as _;
            builder.with_default_menu(false);
        }));
    }

    eframe::run_native(
        "mde",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, files, started)))),
    )
}
