//! Classify journal files and print per-channel statistics.
//!
//! cargo run --release -p uoj-core --example classify -- <file-or-folder>... [--show CHANNEL] [--query Q]

use std::path::PathBuf;
use std::time::Instant;

use uoj_core::{Channel, Filter, Pipeline, Query, Store, View};

fn main() {
    let mut paths = Vec::new();
    let mut show: Option<Channel> = None;
    let mut query = String::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--show" => show = args.next().and_then(|c| Channel::parse(&c)),
            "--query" => query = args.next().unwrap_or_default(),
            _ => paths.push(PathBuf::from(a)),
        }
    }
    let mut files: Vec<PathBuf> = Vec::new();
    for p in paths {
        if p.is_dir() {
            let mut v: Vec<PathBuf> = std::fs::read_dir(&p)
                .unwrap()
                .flatten()
                .map(|e| e.path())
                .collect();
            v.sort();
            files.extend(v);
        } else {
            files.push(p);
        }
    }
    let t0 = Instant::now();
    let mut bytes = 0usize;
    let mut pipeline = Pipeline::default();
    pipeline.begin_history();
    let mut cursors = Vec::new();
    for (i, f) in files.iter().enumerate() {
        let data = std::fs::read(f).expect("read");
        bytes += data.len();
        let text = String::from_utf8_lossy(&data);
        let mut c = pipeline.open_file(i as u16, f.clone());
        pipeline.process_chunk(&mut c, &text);
        cursors.push(c);
    }
    pipeline.finish_history();
    let batch = pipeline.take_batch();
    let t_parse = t0.elapsed();
    let mut store = Store::new();
    store.apply(batch);
    let t_store = t0.elapsed();

    println!(
        "{} files, {:.1} MB, {} entries",
        files.len(),
        bytes as f64 / 1e6,
        store.len()
    );
    println!(
        "parse+classify: {:?}  (+store {:?})",
        t_parse,
        t_store - t_parse
    );
    let counts = store.channel_counts();
    for c in Channel::ALL {
        println!("  {:<9} {:>8}", c.label(), counts[c.index()]);
    }
    println!("people: {}", store.people().len());

    let f = Filter {
        search: Query::parse(&query),
        ..Default::default()
    };
    let mut v = View::new();
    let t1 = Instant::now();
    v.sync(&store, &f);
    println!(
        "filter '{}' (dups hidden): {} rows in {:?}",
        query,
        v.len(),
        t1.elapsed()
    );

    if let Some(ch) = show {
        let mut seen = std::collections::HashSet::new();
        for e in store.entries().iter().filter(|e| e.channel == ch) {
            let line = format!("{}: {}", store.speaker(e), store.text(e));
            if seen.insert(line.clone()) {
                println!("  [{}] {}", uoj_core::time::hm(e.time), line);
            }
            if seen.len() > 120 {
                break;
            }
        }
    }
}
