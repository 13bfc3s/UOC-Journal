//! Finding installed fonts and reading their display names.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontEntry {
    /// Full name from the font file, e.g. `DejaVu Sans Bold`.
    pub name: String,
    pub path: PathBuf,
}

fn is_font_file(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| matches!(e.to_ascii_lowercase().as_str(), "ttf" | "otf" | "ttc"))
        .unwrap_or(false)
}

/// Read a font's full name (falling back to family + style, then the file name).
pub fn describe(path: &Path) -> Option<FontEntry> {
    let meta = std::fs::metadata(path).ok()?;
    // Very large files are CJK collections; skip reading them during scans.
    if meta.len() > 40 * 1024 * 1024 {
        return None;
    }
    let data = std::fs::read(path).ok()?;
    let face = ttf_parser::Face::parse(&data, 0).ok()?;
    // Prefer the US-English name; fonts often carry translated names too.
    let get = |id: u16| {
        let pick = |english: bool| {
            face.names()
                .into_iter()
                .filter(|n| n.name_id == id)
                .filter(|n| !english || n.language() == ttf_parser::Language::English_UnitedStates)
                .find_map(|n| n.to_string())
                .filter(|s| !s.trim().is_empty())
        };
        pick(true).or_else(|| pick(false))
    };
    let name = get(ttf_parser::name_id::FULL_NAME)
        .or_else(|| {
            let fam = get(ttf_parser::name_id::TYPOGRAPHIC_FAMILY)
                .or_else(|| get(ttf_parser::name_id::FAMILY))?;
            let sub = get(ttf_parser::name_id::TYPOGRAPHIC_SUBFAMILY)
                .or_else(|| get(ttf_parser::name_id::SUBFAMILY))
                .unwrap_or_default();
            Some(format!("{fam} {sub}").trim().to_string())
        })
        .or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()))?;
    Some(FontEntry {
        name,
        path: path.to_path_buf(),
    })
}

fn font_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/usr/share/fonts"),
        PathBuf::from("/usr/local/share/fonts"),
        PathBuf::from("/run/host/fonts"), // Flatpak
        PathBuf::from("/Library/Fonts"),
        PathBuf::from("/System/Library/Fonts"),
    ];
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        dirs.push(home.join(".local/share/fonts"));
        dirs.push(home.join(".fonts"));
        dirs.push(home.join("Library/Fonts"));
    }
    if let Some(x) = std::env::var_os("XDG_DATA_HOME") {
        dirs.push(PathBuf::from(x).join("fonts"));
    }
    if let Some(w) = std::env::var_os("WINDIR") {
        dirs.push(PathBuf::from(w).join("Fonts"));
    }
    dirs
}

fn walk(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    if depth == 0 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for ent in rd.flatten() {
        let p = ent.path();
        if p.is_dir() {
            walk(&p, depth - 1, out);
        } else if is_font_file(&p) {
            out.push(p);
        }
    }
}

/// All installed fonts, sorted by name, one entry per distinct name.
pub fn scan_system_fonts() -> Vec<FontEntry> {
    let mut files = Vec::new();
    for d in font_dirs() {
        walk(&d, 6, &mut files);
    }
    files.sort();
    files.dedup();
    let mut out: Vec<FontEntry> = files.iter().filter_map(|p| describe(p)).collect();
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out.dedup_by(|a, b| a.name == b.name);
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn names_are_english_when_available() {
        let p = std::path::Path::new("/usr/share/fonts/truetype/freefont/FreeMonoOblique.ttf");
        if let Some(f) = super::describe(p) {
            assert!(f.name.is_ascii(), "{}", f.name);
        }
    }
}
