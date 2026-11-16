#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cli;
mod window;
mod boxes;
mod catalog;
mod chain;
mod controllers;
mod editors;
mod library;
mod device;
mod devpreset;
mod hlx;
mod msgpack;
mod proto;
mod realnames;
mod nam;
mod irs;
mod selftest;
mod modelinfo;
mod icons;
#[cfg(test)]
mod fakepedal;
mod files;
mod folders;
mod backup;
mod tonelib;
mod session;
mod snapshots;
mod theme;
mod units;
mod wire;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if cli::run(&args) {
        return Ok(());
    }
    let screenshot = args
        .iter()
        .position(|a| a == "--screenshot")
        .and_then(|i| args.get(i + 1))
        .map(std::path::PathBuf::from);


    let mut viewport = egui::ViewportBuilder::default()
        .with_title("ToneStrand")
        .with_app_id("tonestrand")
        .with_inner_size([1320.0, 820.0])
        .with_min_inner_size([960.0, 600.0]);
    if let Some(icon) = theme::window_icon() {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }
    // Filling the screen happens once the window is up (see App::update): asking while it is
    // created leaves it flagged maximized but at its first size on some Windows setups.
    if screenshot.is_some() {
        // Open quietly behind other windows; it keeps drawing there.
        viewport = viewport.with_active(false);
        #[cfg(windows)]
        {
            viewport = viewport.with_window_level(egui::WindowLevel::AlwaysOnBottom);
        }
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    let result = eframe::run_native(
        "ToneStrand",
        options,
        Box::new(move |cc| Ok(Box::new(window::App::new(cc, screenshot)))),
    );
    // Background helpers (network search, a pedal link mid-answer) must not
    // keep the program alive once its window is closed.
    if result.is_ok() {
        std::process::exit(0);
    }
    result
}
