//! UOC Journal — a fast, searchable, themeable journal viewer for ClassicUO
//! based Ultima Online clients (Outlands and others).

mod app;
mod config;
mod logview;
mod pane;
mod settings_ui;
mod theme;

use eframe::egui;

fn icon() -> egui::IconData {
    // A small scroll glyph drawn procedurally so no image file is needed.
    let size = 64u32;
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let i = ((y * size + x) * 4) as usize;
            let (fx, fy) = (x as f32, y as f32);
            let paper = (12.0..52.0).contains(&fx) && (8.0..56.0).contains(&fy);
            let roll =
                ((fy - 8.0).abs() < 4.0 || (fy - 56.0).abs() < 4.0) && (8.0..56.0).contains(&fx);
            let line = paper
                && (18.0..46.0).contains(&fx)
                && (16.0..50.0).contains(&fy)
                && (y - 16) % 8 < 2;
            let c: [u8; 4] = if line {
                [90, 62, 30, 255]
            } else if roll {
                [196, 150, 70, 255]
            } else if paper {
                [238, 222, 186, 255]
            } else {
                [0, 0, 0, 0]
            };
            rgba[i..i + 4].copy_from_slice(&c);
        }
    }
    egui::IconData {
        rgba,
        width: size,
        height: size,
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("UOC Journal")
            .with_app_id("uoc-journal")
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([420.0, 280.0])
            .with_icon(icon()),
        persist_window: true,
        ..Default::default()
    };
    eframe::run_native(
        "UOC Journal",
        options,
        Box::new(|cc| Ok(Box::new(app::JournalApp::new(cc)))),
    )
}
