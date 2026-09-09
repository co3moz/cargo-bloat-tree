use std::io::{Read, Write};
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
        Some(path) => {
            std::fs::read_to_string(path).with_context(|| format!("failed to read `{path}`"))?
        }
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

fn cargo() -> std::ffi::OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into())
}

/// `cargo bloat` is a separate binary, and `cargo install` does not install the
/// binaries of a dependency, so it has to be there before the first build.
fn ensure_cargo_bloat(args: &Args) -> Result<()> {
    let exe = format!("cargo-bloat{}", std::env::consts::EXE_SUFFIX);
    let found = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).any(|dir| dir.join(&exe).is_file()))
        .unwrap_or(false);
    if found {
        return Ok(());
    }

    if !args.install_cargo_bloat && !ask_to_install()? {
        bail!(
            "cargo bloat is not installed. Run `cargo install cargo-bloat`, \
             or pass --install-cargo-bloat to let this tool do it"
        );
    }

    eprintln!("installing cargo-bloat...");
    let status = Command::new(cargo())
        .args(["install", "cargo-bloat", "--locked"])
        .status()
        .context("failed to run `cargo install cargo-bloat`")?;
    if !status.success() {
        bail!("`cargo install cargo-bloat` failed with {status}");
    }
    Ok(())
}

fn ask_to_install() -> Result<bool> {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() {
        return Ok(false);
    }
    eprint!("cargo-bloat is not installed, install it now? [y/N] ");
    std::io::stderr().flush().ok();
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .context("failed to read the answer")?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
}

fn run(args: &Args) -> Result<String> {
    ensure_cargo_bloat(args)?;

    let mut cmd = Command::new(cargo());
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
        .context("failed to run `cargo bloat`")?;
    if !out.status.success() {
        bail!("`cargo bloat` failed with {}", out.status);
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
