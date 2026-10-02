//! UOC Journal — a fast, searchable, themeable journal viewer for ClassicUO
//! based Ultima Online clients (Outlands and others).

mod app;
mod config;
mod fonts;
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

/// The app lives behind a mutex so the settings window, a deferred viewport
/// painted outside the journal window's pass, can reach it too.
struct Shell(std::sync::Arc<std::sync::Mutex<app::JournalApp>>);

fn lock(app: &std::sync::Mutex<app::JournalApp>) -> std::sync::MutexGuard<'_, app::JournalApp> {
    app.lock().unwrap_or_else(|e| e.into_inner())
}

impl eframe::App for Shell {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        eframe::App::ui(&mut *lock(&self.0), ui, frame);
    }
}

/// Set in the child process started by [`supervise`].
const CHILD_ENV: &str = "UOC_JOURNAL_CHILD";

fn unix_time() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Append a report to `crash.log` in the settings folder.
fn write_crash_log(report: &str) {
    use std::io::Write;
    let dir = config::config_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("crash.log"))
    {
        let _ = f.write_all(report.as_bytes());
    }
}

/// Print panics with a backtrace. Without a supervising parent the report also
/// goes straight to `crash.log`; otherwise the parent records it from stderr.
fn install_crash_log() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let bt = std::backtrace::Backtrace::force_capture();
        let report = format!(
            "UOC Journal {} panicked (unix time {})\n{info}\n\n{bt}\n\n",
            env!("CARGO_PKG_VERSION"),
            unix_time()
        );
        if std::env::var_os(CHILD_ENV).is_none() {
            write_crash_log(&report);
        }
        eprint!("{report}");
        default(info);
    }));
}

/// Run the app in a child process and log how it died. Native crashes
/// (graphics driver, X11/Wayland) kill the process without running the panic
/// hook, so only a parent can see them. Returns `None` when the app should run
/// in this process instead.
fn supervise() -> Option<i32> {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::process::ExitStatusExt;
    use std::process::{Command, Stdio};

    if std::env::var_os(CHILD_ENV).is_some()
        || std::env::var_os("UOC_JOURNAL_NO_WATCHDOG").is_some()
    {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let mut child = Command::new(exe)
        .args(std::env::args_os().skip(1))
        .env(CHILD_ENV, "1")
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let stderr = child.stderr.take()?;
    let reader = std::thread::spawn(move || {
        // Pass stderr through and keep its tail for the report.
        let mut tail = std::collections::VecDeque::with_capacity(400);
        let mut err = std::io::stderr();
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let _ = writeln!(err, "{line}");
            if tail.len() == 400 {
                tail.pop_front();
            }
            tail.push_back(line);
        }
        tail
    });
    let status = child.wait().ok()?;
    let tail = reader.join().unwrap_or_default();
    if status.success() {
        return Some(0);
    }
    let reason = match (status.code(), status.signal()) {
        (Some(code), _) => Some(format!("exited with code {code}")),
        (None, Some(sig)) => {
            let name = match sig {
                4 => "SIGILL, illegal instruction",
                5 => "SIGTRAP",
                6 => "SIGABRT, aborted",
                7 => "SIGBUS, bus error",
                8 => "SIGFPE, arithmetic error",
                9 => "SIGKILL: force-quit by the desktop because the window stopped responding, or out of memory",
                11 => "SIGSEGV, segmentation fault",
                // Normal ways of being asked to quit (logout, Ctrl+C, kill).
                1 | 2 | 15 => "",
                _ => "unexpected signal",
            };
            (!name.is_empty()).then(|| format!("killed by signal {sig} ({name})"))
        }
        _ => None,
    };
    if let Some(reason) = reason {
        let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".into());
        let report = format!(
            "UOC Journal {} {reason} (unix time {}, session {session})\n\
             --- last {} lines of output ---\n{}\n\n",
            env!("CARGO_PKG_VERSION"),
            unix_time(),
            tail.len(),
            tail.into_iter().collect::<Vec<_>>().join("\n")
        );
        write_crash_log(&report);
    }
    Some(
        status
            .code()
            .or(status.signal().map(|s| 128 + s))
            .unwrap_or(1),
    )
}

fn main() -> eframe::Result {
    if let Some(code) = supervise() {
        std::process::exit(code);
    }
    install_crash_log();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("UOC Journal")
            .with_app_id("uoc-journal")
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([420.0, 280.0])
            .with_icon(icon()),
        persist_window: true,
        // With vsync on, a buffer swap on Wayland waits for the compositor's
        // frame callback, and compositors stop sending those to windows that
        // are hidden behind others. The Settings window is drawn on the same
        // thread, so it froze too until the desktop offered to force-quit
        // (SIGKILL). The app only repaints when something changes, so it
        // doesn't need vsync.
        glow_options: eframe::egui_glow::GlowConfiguration {
            vsync: false,
            ..Default::default()
        },
        ..Default::default()
    };
    eframe::run_native(
        "UOC Journal",
        options,
        Box::new(|cc| {
            let app = std::sync::Arc::new(std::sync::Mutex::new(app::JournalApp::new(cc)));
            lock(&app).this = std::sync::Arc::downgrade(&app);
            Ok(Box::new(Shell(app)))
        }),
    )
}
