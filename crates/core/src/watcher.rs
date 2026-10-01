//! Background thread that finds journal files, loads recent history and tails
//! every file that is still being written (one per running client).
//!
//! Files are polled rather than watched with OS notifications: ClassicUO keeps
//! the journal open for the whole session (natively or under Wine/Proton), and
//! a `read()` on our own handle always sees freshly flushed bytes no matter which
//! filesystem or compatibility layer wrote them. Polling a handful of open
//! handles every few dozen milliseconds costs next to nothing.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use rustc_hash::FxHashSet;

use crate::classify::{compile_rules, UserRule};
use crate::parse;
use crate::pipeline::{Batch, FileCursor, Pipeline};

#[derive(Clone, Debug, PartialEq)]
pub struct WatchConfig {
    /// Folder with `*_journal.txt` files (ClassicUO: `Data/Client/JournalLogs`).
    pub folder: Option<PathBuf>,
    /// Hours of history to load at startup, counted back from the newest file.
    pub history_hours: u32,
    /// Upper bound on the number of history files loaded at startup.
    pub max_history_files: usize,
    pub rules: Vec<UserRule>,
    /// How often open files are checked for new lines.
    pub poll_ms: u64,
}

impl Default for WatchConfig {
    fn default() -> Self {
        WatchConfig {
            folder: None,
            history_hours: 12,
            max_history_files: 40,
            rules: Vec::new(),
            poll_ms: 40,
        }
    }
}

pub enum Command {
    Configure(WatchConfig),
    Reload,
    /// Load (and follow) specific files in addition to the folder.
    OpenFiles(Vec<PathBuf>),
    /// Load every journal file in the folder.
    LoadAll,
    Shutdown,
}

#[derive(Clone, Debug, Default)]
pub struct FileStatus {
    pub path: PathBuf,
    pub name: String,
    pub bytes: u64,
    pub character: Option<String>,
    /// Still being written by a client.
    pub active: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub files: Vec<FileStatus>,
    pub loading: Option<String>,
    pub errors: Vec<String>,
    /// Milliseconds the last history load took.
    pub load_ms: Option<u64>,
}

pub enum Event {
    Batch(Batch),
    Status(Status),
}

pub struct Watcher {
    tx: Sender<Command>,
    rx: Receiver<Event>,
    handle: Option<JoinHandle<()>>,
}

impl Watcher {
    pub fn spawn(config: WatchConfig, wake: impl Fn() + Send + 'static) -> Watcher {
        let (tx, crx) = mpsc::channel();
        let (etx, rx) = mpsc::channel();
        let handle = std::thread::Builder::new()
            .name("journal-watcher".into())
            .spawn(move || Worker::new(config, etx, Box::new(wake)).run(crx))
            .expect("spawn watcher thread");
        Watcher {
            tx,
            rx,
            handle: Some(handle),
        }
    }

    pub fn send(&self, cmd: Command) {
        let _ = self.tx.send(cmd);
    }

    pub fn try_recv(&self) -> Option<Event> {
        self.rx.try_recv().ok()
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Shutdown);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

struct Tracked {
    cursor: FileCursor,
    file: Option<File>,
    offset: u64,
    partial: Vec<u8>,
    last_growth: Instant,
    active: bool,
}

struct Worker {
    cfg: WatchConfig,
    tx: Sender<Event>,
    wake: Box<dyn Fn() + Send>,
    pipeline: Pipeline,
    files: Vec<Tracked>,
    known: FxHashSet<PathBuf>,
    extra: Vec<PathBuf>,
    errors: Vec<String>,
    last_scan: Instant,
    last_status: Instant,
    status_dirty: bool,
    load_ms: Option<u64>,
    buf: Vec<u8>,
}

/// Files that have not grown for this long are no longer shown as active.
const ACTIVE_WINDOW: Duration = Duration::from_secs(15 * 60);

impl Worker {
    fn new(cfg: WatchConfig, tx: Sender<Event>, wake: Box<dyn Fn() + Send>) -> Self {
        Worker {
            cfg,
            tx,
            wake,
            pipeline: Pipeline::default(),
            files: Vec::new(),
            known: FxHashSet::default(),
            extra: Vec::new(),
            errors: Vec::new(),
            last_scan: Instant::now(),
            last_status: Instant::now(),
            status_dirty: true,
            load_ms: None,
            buf: Vec::with_capacity(64 * 1024),
        }
    }

    fn run(mut self, rx: Receiver<Command>) {
        self.full_load(false);
        loop {
            let poll = Duration::from_millis(self.cfg.poll_ms.clamp(10, 1000));
            match rx.recv_timeout(poll) {
                Ok(Command::Shutdown) | Err(RecvTimeoutError::Disconnected) => return,
                Ok(Command::Configure(cfg)) => {
                    let reload = cfg.folder != self.cfg.folder
                        || cfg.rules != self.cfg.rules
                        || cfg.history_hours != self.cfg.history_hours
                        || cfg.max_history_files != self.cfg.max_history_files;
                    self.cfg = cfg;
                    if reload {
                        self.full_load(false);
                    }
                }
                Ok(Command::Reload) => self.full_load(false),
                Ok(Command::LoadAll) => self.full_load(true),
                Ok(Command::OpenFiles(paths)) => {
                    for p in paths {
                        if !self.extra.contains(&p) {
                            self.extra.push(p);
                        }
                    }
                    self.full_load(false);
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            self.tick();
        }
    }

    fn send(&self, ev: Event) {
        if self.tx.send(ev).is_ok() {
            (self.wake)();
        }
    }

    fn flush(&mut self) {
        let batch = self.pipeline.take_batch();
        if !batch.is_empty() {
            self.status_dirty |= !batch.sessions.is_empty();
            self.send(Event::Batch(batch));
        }
    }

    fn status(&mut self) {
        let sessions = self.pipeline.sessions();
        let files = self
            .files
            .iter()
            .map(|t| {
                let character = sessions
                    .iter()
                    .rev()
                    .find(|s| s.file == t.cursor.file && s.character.is_some())
                    .and_then(|s| s.character.clone());
                FileStatus {
                    path: t.cursor.path.clone(),
                    name: t.cursor.name.clone(),
                    bytes: t.offset,
                    character,
                    active: t.active,
                }
            })
            .collect();
        let st = Status {
            files,
            loading: None,
            errors: self.errors.clone(),
            load_ms: self.load_ms,
        };
        self.send(Event::Status(st));
        self.status_dirty = false;
        self.last_status = Instant::now();
    }

    /// Journal files in the configured folder as (path, start minute, last-write
    /// minute), oldest start first.
    fn discover(&mut self) -> Vec<(PathBuf, u32, u32)> {
        let mut out = Vec::new();
        if let Some(dir) = self.cfg.folder.clone() {
            match std::fs::read_dir(&dir) {
                Ok(rd) => {
                    for ent in rd.flatten() {
                        let name = ent.file_name().to_string_lossy().into_owned();
                        if !parse::is_journal_file_name(&name) {
                            continue;
                        }
                        let modified = ent
                            .metadata()
                            .ok()
                            .and_then(|m| m.modified().ok())
                            .map(system_minutes);
                        let start = parse::file_start_minutes(&name).or(modified).unwrap_or(0);
                        out.push((ent.path(), start, modified.unwrap_or(start)));
                    }
                }
                Err(e) => self
                    .errors
                    .push(format!("Cannot read folder {}: {e}", dir.display())),
            }
        }
        out.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        out
    }

    fn full_load(&mut self, everything: bool) {
        let started = Instant::now();
        self.errors.clear();
        let (rules, errs) = compile_rules(&self.cfg.rules);
        self.errors.extend(errs);
        self.pipeline.reset();
        self.pipeline.set_rules(rules);
        self.files.clear();
        self.known.clear();

        let found = self.discover();
        for (p, ..) in &found {
            self.known.insert(p.clone());
        }
        // The window is anchored at the most recent write, and a file counts if it
        // was still being written inside the window (sessions can last for days).
        let newest = found.iter().map(|f| f.2).max().unwrap_or(0);
        let window = self.cfg.history_hours.saturating_mul(60);
        let mut chosen: Vec<PathBuf> = if everything {
            found.iter().map(|f| f.0.clone()).collect()
        } else {
            let recent: Vec<PathBuf> = found
                .iter()
                .filter(|f| newest.saturating_sub(f.2) <= window)
                .map(|f| f.0.clone())
                .collect();
            let skip = recent
                .len()
                .saturating_sub(self.cfg.max_history_files.max(1));
            recent.into_iter().skip(skip).collect()
        };
        // Always follow the newest file even with a zero-hour window.
        if let Some(last) = found.last() {
            if !chosen.contains(&last.0) {
                chosen.push(last.0.clone());
            }
        }
        for p in &self.extra {
            if !chosen.contains(p) {
                chosen.push(p.clone());
            }
            self.known.insert(p.clone());
        }

        let total: u64 = chosen
            .iter()
            .filter_map(|p| std::fs::metadata(p).ok())
            .map(|m| m.len())
            .sum();
        let msg = format!(
            "Loading {} file(s), {:.1} MB…",
            chosen.len(),
            total as f64 / 1_048_576.0
        );
        self.send(Event::Status(Status {
            loading: Some(msg),
            ..Default::default()
        }));

        self.pipeline.begin_history();
        for path in chosen {
            self.add_file(path);
        }
        self.pipeline.finish_history();
        for t in &mut self.files {
            t.cursor.forget_recent();
        }
        self.flush();
        self.load_ms = Some(started.elapsed().as_millis() as u64);
        self.last_scan = Instant::now();
        self.status();
    }

    fn add_file(&mut self, path: PathBuf) {
        let id = self.files.len().min(u16::MAX as usize) as u16;
        let cursor = self.pipeline.open_file(id, path.clone());
        let file = match File::open(&path) {
            Ok(f) => Some(f),
            Err(e) => {
                self.errors
                    .push(format!("Cannot open {}: {e}", path.display()));
                None
            }
        };
        // How long ago the client last wrote to it decides whether it is "live".
        let age = std::fs::metadata(&path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .unwrap_or(ACTIVE_WINDOW * 2);
        let mut t = Tracked {
            cursor,
            file,
            offset: 0,
            partial: Vec::new(),
            last_growth: Instant::now(),
            active: false,
        };
        Self::read_new(&mut self.pipeline, &mut t, &mut self.buf, true);
        t.last_growth = Instant::now()
            .checked_sub(age)
            .unwrap_or_else(|| Instant::now() - Duration::from_secs(1));
        t.active = age < ACTIVE_WINDOW;
        self.files.push(t);
        self.status_dirty = true;
    }

    /// Read whatever was appended since last time and feed complete lines.
    /// Returns true when new bytes arrived.
    fn read_new(
        pipeline: &mut Pipeline,
        t: &mut Tracked,
        buf: &mut Vec<u8>,
        initial: bool,
    ) -> bool {
        let Some(file) = t.file.as_mut() else {
            return false;
        };
        buf.clear();
        match file.read_to_end(buf) {
            Ok(0) => return false,
            Ok(_) => {}
            Err(_) => return false,
        }
        t.offset += buf.len() as u64;
        t.last_growth = Instant::now();
        t.active = true;
        let mut data = std::mem::take(&mut t.partial);
        if data.is_empty() {
            std::mem::swap(&mut data, buf);
        } else {
            data.extend_from_slice(buf);
        }
        let skip_bom = if initial && data.starts_with(&[0xEF, 0xBB, 0xBF]) {
            3
        } else {
            0
        };
        match memchr::memrchr(b'\n', &data) {
            Some(last) => {
                let complete = &data[skip_bom..=last];
                let text = String::from_utf8_lossy(complete);
                pipeline.process_chunk(&mut t.cursor, &text);
                t.partial = data[last + 1..].to_vec();
            }
            None => t.partial = data[skip_bom..].to_vec(),
        }
        if buf.capacity() == 0 {
            *buf = Vec::with_capacity(64 * 1024);
        }
        true
    }

    fn tick(&mut self) {
        let mut any = false;
        for t in &mut self.files {
            if Self::read_new(&mut self.pipeline, t, &mut self.buf, false) {
                any = true;
            } else if !t.partial.is_empty() && t.last_growth.elapsed() > Duration::from_millis(750)
            {
                // A line without terminator that is not going to be completed.
                let text = String::from_utf8_lossy(&std::mem::take(&mut t.partial)).into_owned();
                self.pipeline.process_line(&mut t.cursor, &text);
                any = true;
            }
        }
        if any {
            self.flush();
        }

        if self.last_scan.elapsed() >= Duration::from_millis(1000) {
            self.last_scan = Instant::now();
            self.check_files();
        }
        if self.status_dirty && self.last_status.elapsed() >= Duration::from_millis(500) {
            self.status();
        }
    }

    /// New files appearing (a client was started), truncation, activity changes.
    fn check_files(&mut self) {
        let found = self.discover();
        let mut added = false;
        for (p, ..) in found {
            if self.known.insert(p.clone()) {
                self.add_file(p);
                added = true;
            }
        }
        if added {
            self.flush();
        }
        for t in &mut self.files {
            let was = t.active;
            t.active = t.last_growth.elapsed() < ACTIVE_WINDOW;
            if let Some(f) = t.file.as_mut() {
                if let Ok(m) = f.metadata() {
                    if m.len() < t.offset {
                        // Truncated or replaced: start over from the top.
                        let _ = f.seek(SeekFrom::Start(0));
                        t.offset = 0;
                        t.partial.clear();
                    }
                }
            }
            if was != t.active {
                self.status_dirty = true;
            }
        }
    }
}

fn system_minutes(t: SystemTime) -> u32 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| (d.as_secs() / 60) as u32)
        .unwrap_or(0)
}

/// Places a ClassicUO install (and therefore `Data/Client/JournalLogs`) is
/// likely to live under: native installs, Wine, Lutris, Bottles and Steam/Proton
/// prefixes on Linux, and the usual Windows locations.
fn candidate_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    let home = std::env::var_os("HOME").map(PathBuf::from);

    // Wine prefixes, whose drive_c is searched like a Windows machine.
    let mut prefixes: Vec<PathBuf> = Vec::new();
    if let Some(h) = &home {
        prefixes.push(h.join(".wine"));
        let groups = [
            h.join("Games"),
            h.join(".local/share/lutris/prefixes"),
            h.join(".local/share/bottles/bottles"),
            h.join(".var/app/com.usebottles.bottles/data/bottles/bottles"),
            h.join(".steam/steam/steamapps/compatdata"),
            h.join(".local/share/Steam/steamapps/compatdata"),
            h.join(".var/app/com.valvesoftware.Steam/.local/share/Steam/steamapps/compatdata"),
        ];
        for g in groups {
            if let Ok(rd) = std::fs::read_dir(&g) {
                for ent in rd.flatten() {
                    let p = ent.path();
                    if p.join("drive_c").is_dir() {
                        prefixes.push(p.clone());
                    }
                    if p.join("pfx/drive_c").is_dir() {
                        prefixes.push(p.join("pfx"));
                    }
                }
            }
        }
    }
    if let Some(wp) = std::env::var_os("WINEPREFIX") {
        prefixes.push(PathBuf::from(wp));
    }
    for pfx in prefixes {
        let c = pfx.join("drive_c");
        if !c.is_dir() {
            continue;
        }
        roots.push(c.join("Program Files"));
        roots.push(c.join("Program Files (x86)"));
        roots.push(c.join("Games"));
        roots.push(c.clone());
        if let Ok(rd) = std::fs::read_dir(c.join("users")) {
            for u in rd.flatten() {
                let u = u.path();
                for sub in [
                    "AppData/Local/Programs",
                    "AppData/Local",
                    "AppData/Roaming",
                    "Desktop",
                    "Documents",
                ] {
                    roots.push(u.join(sub));
                }
            }
        }
    }

    // Native installs (ClassicUO runs on Linux directly).
    if let Some(h) = &home {
        for sub in [
            "",
            "Games",
            "Applications",
            "Downloads",
            ".local/share",
            ".local/opt",
            "opt",
        ] {
            roots.push(if sub.is_empty() {
                h.clone()
            } else {
                h.join(sub)
            });
        }
    }
    if let Some(x) = std::env::var_os("XDG_DATA_HOME") {
        roots.push(PathBuf::from(x));
    }
    roots.push(PathBuf::from("/opt"));

    // Windows.
    for var in [
        "LOCALAPPDATA",
        "APPDATA",
        "PROGRAMFILES",
        "PROGRAMFILES(X86)",
        "USERPROFILE",
        "PUBLIC",
    ] {
        if let Some(v) = std::env::var_os(var) {
            let p = PathBuf::from(v);
            roots.push(p.join("Programs"));
            roots.push(p.join("Games"));
            roots.push(p.join("Documents"));
            roots.push(p.join("Desktop"));
            roots.push(p);
        }
    }
    if cfg!(windows) {
        for drive in ["C:\\", "D:\\", "E:\\"] {
            roots.push(PathBuf::from(drive));
            roots.push(PathBuf::from(drive).join("Games"));
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            roots.push(dir.to_path_buf());
            if let Some(up) = dir.parent() {
                roots.push(up.to_path_buf());
            }
        }
    }
    roots
}

/// Look for ClassicUO `JournalLogs` folders in the usual install locations.
/// Results are ordered by most recently written journal first.
pub fn detect_journal_dirs() -> Vec<PathBuf> {
    let interesting = |name: &str| {
        let n = name.to_ascii_lowercase();
        [
            "uo",
            "ultima",
            "outlands",
            "classicuo",
            "razor",
            "britannia",
        ]
        .iter()
        .any(|k| n.contains(k))
    };
    let mut found: Vec<(PathBuf, SystemTime)> = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut seen = FxHashSet::default();
    for root in candidate_roots() {
        if Instant::now() > deadline {
            break;
        }
        if !root.is_dir() || !seen.insert(root.clone()) {
            continue;
        }
        let Ok(rd) = std::fs::read_dir(&root) else {
            continue;
        };
        for ent in rd.flatten() {
            if Instant::now() > deadline {
                break;
            }
            let name = ent.file_name().to_string_lossy().into_owned();
            // `Path::is_dir` follows symlinks (prefixes on other disks are common).
            if interesting(&name) && ent.path().is_dir() {
                search_journal_dirs(&ent.path(), 6, deadline, &mut found);
            }
        }
    }
    found.sort_by_key(|f| std::cmp::Reverse(f.1));
    let mut out: Vec<PathBuf> = Vec::new();
    for (p, _) in found {
        let p = p.canonicalize().unwrap_or(p);
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

fn search_journal_dirs(
    dir: &Path,
    depth: u32,
    deadline: Instant,
    found: &mut Vec<(PathBuf, SystemTime)>,
) {
    if depth == 0 || Instant::now() > deadline {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for ent in rd.flatten() {
        if !ent.path().is_dir() {
            continue;
        }
        let name = ent.file_name().to_string_lossy().to_ascii_lowercase();
        if name == "journallogs" {
            let newest = newest_journal(&ent.path()).unwrap_or(SystemTime::UNIX_EPOCH);
            found.push((ent.path(), newest));
        } else if !matches!(
            name.as_str(),
            "node_modules"
                | ".git"
                | "cache"
                | "temp"
                | "logs"
                | "screenshots"
                | "music"
                | "sound"
                | "maps"
                | "dosdevices"
        ) {
            search_journal_dirs(&ent.path(), depth - 1, deadline, found);
        }
    }
}

/// Modification time of the newest journal in `dir`.
pub fn newest_journal(dir: &Path) -> Option<SystemTime> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| parse::is_journal_file_name(&e.file_name().to_string_lossy()))
        .filter_map(|e| e.metadata().ok()?.modified().ok())
        .max()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use std::io::Write;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("uoj-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn drain(w: &Watcher, store: &mut Store, until: impl Fn(&Store) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            while let Some(ev) = w.try_recv() {
                if let Event::Batch(b) = ev {
                    assert!(store.apply(b));
                }
            }
            if until(store) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out; store has {} entries", store.len());
    }

    #[test]
    fn detects_linux_install_layouts() {
        let home = tmpdir("home");
        let dirs = [
            ".wine/drive_c/Program Files (x86)/Ultima Online Outlands/ClassicUO/Data/Client/JournalLogs",
            "Games/outlands/drive_c/users/me/AppData/Local/Programs/UO Outlands/Data/Client/JournalLogs",
            ".local/share/Steam/steamapps/compatdata/123/pfx/drive_c/Games/ClassicUO/Data/Client/JournalLogs",
            "ClassicUO/Data/Client/JournalLogs",
        ];
        for d in dirs {
            let p = home.join(d);
            std::fs::create_dir_all(&p).unwrap();
            std::fs::write(p.join("2026_03_14_18_02_11_journal.txt"), "x\n").unwrap();
        }
        // Noise that must not be picked up.
        std::fs::create_dir_all(home.join("Documents/JournalLogs")).unwrap();
        let old = std::env::var_os("HOME");
        std::env::set_var("HOME", &home);
        let found = detect_journal_dirs();
        match old {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        for d in dirs {
            let want = home.join(d).canonicalize().unwrap();
            assert!(found.contains(&want), "missing {d}: {found:?}");
        }
        assert!(!found.iter().any(|p| p.starts_with(home.join("Documents"))));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn tails_two_clients() {
        let dir = tmpdir("tail");
        let a = dir.join("2026_03_14_18_02_11_journal.txt");
        let b = dir.join("2026_03_14_18_05_40_journal.txt");
        std::fs::write(&a, "[03/14/2026 18:02]  System: Welcome Aldric Thorne!\r\n").unwrap();
        let w = Watcher::spawn(
            WatchConfig {
                folder: Some(dir.clone()),
                poll_ms: 10,
                ..Default::default()
            },
            || {},
        );
        let mut store = Store::new();
        drain(&w, &mut store, |s| s.len() == 1);

        // Appends to the existing file, including a split line.
        let mut f = std::fs::OpenOptions::new().append(true).open(&a).unwrap();
        f.write_all(b"[03/14/2026 18:12]  [Alliance][Lysa Quill]: [OAK] reds near the north bridge\r\n[03/14/2026 18:12]  [Guild][Oswin").unwrap();
        f.flush().unwrap();
        drain(&w, &mut store, |s| s.len() == 2);
        f.write_all(b" Pike]: east wing clear\r\n").unwrap();
        f.flush().unwrap();
        drain(&w, &mut store, |s| s.len() == 3);
        assert_eq!(store.speaker(store.entry(2)), "Oswin Pike");

        // A second client starts.
        std::fs::write(&b, "[03/14/2026 18:05]  System: Welcome Wynn Harrow!\r\n[03/14/2026 18:12]  [Alliance][Lysa Quill]: [OAK] reds near the north bridge\r\n").unwrap();
        drain(&w, &mut store, |s| s.len() == 5);
        assert_eq!(
            store.characters(),
            vec!["Aldric Thorne".to_string(), "Wynn Harrow".to_string()]
        );
        assert!(store.entry(4).has(crate::flags::DUP));
        drop(w);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
