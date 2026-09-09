use std::io::Read;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::cli::Args;

/// The `--message-format json` output of `cargo bloat --crates`.
#[derive(Debug, Deserialize)]
pub struct BloatOutput {
    #[serde(rename = "file-size")]
    pub file_size: u64,
    #[serde(rename = "text-section-size")]
    pub text_size: u64,
    #[serde(default)]
    pub crates: Vec<CrateSize>,
}

#[derive(Debug, Deserialize)]
pub struct CrateSize {
    pub name: String,
    pub size: u64,
}

/// Either runs `cargo bloat` or reads a result that was produced earlier.
pub fn obtain(args: &Args) -> Result<BloatOutput> {
    let raw = match args.bloat_json.as_deref() {
        Some("-") => {
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .context("failed to read cargo bloat JSON from stdin")?;
            buf
        }
        Some(path) => std::fs::read_to_string(path)
            .with_context(|| format!("failed to read `{path}`"))?,
        None => run(args)?,
    };

    if let Some(path) = &args.save_bloat_json {
        std::fs::write(path, &raw).with_context(|| format!("failed to write `{path}`"))?;
    }

    // `cargo bloat` prints the JSON on the last non-empty line; anything a
    // wrapper may have printed before it is ignored.
    let line = raw
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| l.starts_with('{'))
        .context("no JSON object found in the cargo bloat output")?;

    let out: BloatOutput =
        serde_json::from_str(line).context("failed to parse the cargo bloat JSON")?;

    if out.crates.is_empty() {
        bail!(
            "the cargo bloat output contains no per-crate data \
             (it must be produced with `--crates`)"
        );
    }
    Ok(out)
}

fn run(args: &Args) -> Result<String> {
    let mut cmd = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    cmd.arg("bloat")
        .args(["--crates", "-n", "0", "--message-format", "json"]);

    if args.release {
        cmd.arg("--release");
    }
    if args.all_features {
        cmd.arg("--all-features");
    }
    if args.no_default_features {
        cmd.arg("--no-default-features");
    }
    if args.lib {
        cmd.arg("--lib");
    }
    if args.split_std {
        cmd.arg("--split-std");
    }
    if args.locked {
        cmd.arg("--locked");
    }
    if args.frozen {
        cmd.arg("--frozen");
    }
    for (flag, value) in [
        ("--profile", &args.profile),
        ("--target", &args.target),
        ("--features", &args.features),
        ("--package", &args.package),
        ("--bin", &args.bin),
        ("--example", &args.example),
        ("--test", &args.test),
        ("--target-dir", &args.target_dir),
        ("--jobs", &args.jobs),
    ] {
        if let Some(v) = value {
            cmd.arg(flag).arg(v);
        }
    }
    // `cargo bloat` has no --manifest-path, so run it next to the manifest.
    if let Some(path) = &args.manifest_path {
        let dir = std::path::Path::new(path)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_path_buf());
        if let Some(dir) = dir {
            cmd.current_dir(dir);
        }
    }
    if args.offline {
        cmd.env("CARGO_NET_OFFLINE", "true");
    }

    if args.verbose {
        eprintln!("running: {cmd:?}");
    }

    let out = cmd
        .stderr(Stdio::inherit())
        .output()
        .context("failed to run `cargo bloat`, install it with `cargo install cargo-bloat`")?;
    if !out.status.success() {
        bail!("`cargo bloat` failed with {}", out.status);
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
