//! Native entry point for the chess application.
//!
//! This module configures the `eframe` native window and starts
//! [`crate::app::ChessApp`]. It depends only on the top-level app module.

mod app;
mod assets;
mod board;
mod game;
mod promotion;

use app::ChessApp;
use eframe::egui::ViewportBuilder;

/// Starts the native desktop application.
///
/// Returns an `eframe::Result` from the platform integration if window
/// creation or the event loop fails.
fn main() -> eframe::Result<()> {
    // NativeOptions configures the desktop window before egui starts drawing.
    let options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_inner_size([900.0, 720.0])
            .with_min_inner_size([700.0, 520.0])
            .with_title("Chess App"),
        ..Default::default()
    };

    // run_native starts the event loop. The closure creates our app state once,
    // then egui calls `ChessApp::ui` every frame.
    eframe::run_native(
        "Chess App",
        options,
        Box::new(|creation_context| Ok(Box::new(ChessApp::new(creation_context.egui_ctx.clone())))),
    )
}
