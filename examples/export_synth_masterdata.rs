//! Writes a synthetic masterdata set to disk, ready to load.
//!
//! This repository carries no game data, which makes it awkward to try the engine or
//! the HTTP service without first assembling a masterdata directory. The benchmark's
//! deterministic generator already produces tables with the real schema and a
//! realistic shape — 26 characters, 1300 cards, a fully levelled account — so this
//! example writes that set out in the layout the loaders expect.
//!
//! ```text
//! cargo run --release --example export_synth_masterdata -- ./synth
//!
//! ./synth/masterdata/*.json   # the tables
//! ./synth/music_metas.json
//! ./synth/user.json           # a matching account
//! ```
//!
//! The numbers are synthetic, so timings taken against them are for comparing runs
//! with each other, not for predicting behaviour on real data.

#[path = "../benches/synth_masterdata/mod.rs"]
mod synth_masterdata;

use std::path::{Path, PathBuf};

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args
        .next()
        .map_or_else(|| PathBuf::from("synth"), PathBuf::from);
    let seed = match args.next() {
        None => synth_masterdata::DEFAULT_SEED,
        Some(text) => match text.parse::<u64>() {
            Ok(seed) => seed,
            Err(error) => {
                eprintln!("seed {text} is not a number: {error}");
                std::process::exit(1);
            }
        },
    };

    if let Err(error) = export(&out, seed) {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn export(out: &Path, seed: u64) -> Result<(), String> {
    let synth = synth_masterdata::generate(seed);
    let masterdata = out.join("masterdata");
    std::fs::create_dir_all(&masterdata)
        .map_err(|error| format!("creating {} failed: {error}", masterdata.display()))?;

    for (name, json) in &synth.tables {
        write(&masterdata.join(name), json)?;
    }
    write(&out.join("music_metas.json"), &synth.music_metas_json)?;
    write(&out.join("user.json"), &synth.user_json)?;

    println!(
        "wrote {} tables to {}",
        synth.tables.len(),
        masterdata.display()
    );
    println!("  music metas: {}", out.join("music_metas.json").display());
    println!("  user:        {}", out.join("user.json").display());
    println!();
    println!("Serve it with:");
    println!(
        "  allium-deck-server --masterdata synth={} --music-metas synth={}",
        masterdata.display(),
        out.join("music_metas.json").display()
    );
    Ok(())
}

fn write(path: &Path, contents: &str) -> Result<(), String> {
    std::fs::write(path, contents)
        .map_err(|error| format!("writing {} failed: {error}", path.display()))
}
