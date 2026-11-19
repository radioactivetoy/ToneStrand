#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod boxproxy;
mod chain;
mod cli;
mod controllers;
mod editors;
mod folders;
mod icons;
mod library;
mod modelinfo;
mod theme;
mod tonelib;
mod window;

pub use tonestrand_core::{
    backup, boxes, catalog, cmdslots, cmdtool, compare, device, devpreset, files, hlx, irs, msgpack, nam, proto, realnames, routes,
    selftest, session, snapshots, support, units, wire,
};

use eframe::egui;

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if cli::run(&args) {
        return Ok(());
    }
    let screenshot = args.iter().position(|a| a == "--screenshot").and_then(|i| args.get(i + 1)).map(std::path::PathBuf::from);

    // TONESTRAND_SIZE=WxH sets the window size (points) for a picture that needs more room.
    let size = std::env::var("TONESTRAND_SIZE")
        .ok()
        .and_then(|v| v.split_once('x').and_then(|(w, h)| Some([w.parse::<f32>().ok()?, h.parse::<f32>().ok()?])))
        .unwrap_or([1320.0, 820.0]);
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("ToneStrand")
        .with_app_id("tonestrand")
        .with_inner_size(size)
        .with_min_inner_size([960.0, 600.0]);
    // Back where it was left (kept sane: a window never starts tiny or far off screen).
    if screenshot.is_none() {
        if let Some(([x, y, w, h], _)) = boxes::saved_window() {
            if (400.0..=8000.0).contains(&w) && (300.0..=5000.0).contains(&h) {
                viewport = viewport.with_inner_size([w, h]);
                if (-20.0..=6000.0).contains(&x) && (-20.0..=3000.0).contains(&y) {
                    viewport = viewport.with_position([x, y]);
                }
            }
        }
    }
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
    let result = eframe::run_native("ToneStrand", options, Box::new(move |cc| Ok(Box::new(window::App::new(cc, screenshot)))));
    // Background helpers (network search, a pedal link mid-answer) must not
    // keep the program alive once its window is closed.
    if result.is_ok() {
        std::process::exit(0);
    }
    result
}
