use std::collections::HashMap;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::cli::Args;

#[derive(Debug, Deserialize)]
struct Metadata {
    packages: Vec<RawPackage>,
    workspace_members: Vec<String>,
    #[serde(default)]
    workspace_default_members: Vec<String>,
    resolve: Resolve,
}

#[derive(Debug, Deserialize)]
struct RawPackage {
    id: String,
    name: String,
    version: String,
    targets: Vec<Target>,
}

#[derive(Debug, Deserialize)]
struct Target {
    kind: Vec<String>,
    name: String,
}

#[derive(Debug, Deserialize)]
struct Resolve {
    nodes: Vec<Node>,
    #[serde(default)]
    root: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Node {
    id: String,
    deps: Vec<NodeDep>,
}

#[derive(Debug, Deserialize)]
struct NodeDep {
    pkg: String,
    #[serde(default)]
    dep_kinds: Vec<DepKind>,
}

#[derive(Debug, Deserialize)]
struct DepKind {
    #[serde(default)]
    kind: Option<String>,
}

/// A package, indexed by position in [`Graph::pkgs`].
#[derive(Debug, Clone)]
pub struct Pkg {
    pub name: String,
    pub version: String,
    /// The crate name the compiler (and therefore `cargo bloat`) uses.
    pub lib_name: Option<String>,
    pub is_proc_macro: bool,
}

impl Pkg {
    pub fn label(&self) -> String {
        format!("{} v{}", self.name, self.version)
    }
}

/// The resolved dependency graph, reduced to the edges that can contribute
/// code to the analysed artifact.
#[derive(Debug)]
pub struct Graph {
    pub pkgs: Vec<Pkg>,
    pub edges: Vec<Vec<usize>>,
    pub root: usize,
    /// Crate name (as the compiler spells it) -> packages providing it.
    pub by_crate_name: HashMap<String, Vec<usize>>,
}

pub fn load(args: &Args) -> Result<Graph> {
    let meta = run(args)?;
    build(args, meta)
}

fn run(args: &Args) -> Result<Metadata> {
    let mut cmd = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    cmd.args(["metadata", "--format-version", "1"]);

    // Restrict the graph to the platform that was actually built, so that
    // `cfg`-gated dependencies of other platforms do not show up.
    match &args.target {
        Some(t) => {
            cmd.arg("--filter-platform").arg(t);
        }
        None => {
            if let Some(host) = host_triple() {
                cmd.arg("--filter-platform").arg(host);
            }
        }
    }
    if args.all_features {
        cmd.arg("--all-features");
    }
    if args.no_default_features {
        cmd.arg("--no-default-features");
    }
    if let Some(features) = &args.features {
        // cargo bloat takes a space separated list, cargo metadata accepts both.
        cmd.arg("--features").arg(features);
    }
    if let Some(path) = &args.manifest_path {
        cmd.arg("--manifest-path").arg(path);
    }
    if args.locked {
        cmd.arg("--locked");
    }
    if args.offline {
        cmd.arg("--offline");
    }
    if args.frozen {
        cmd.arg("--frozen");
    }

    if args.verbose {
        eprintln!("running: {cmd:?}");
    }

    let out = cmd
        .stderr(Stdio::inherit())
        .output()
        .context("failed to run `cargo metadata`")?;
    if !out.status.success() {
        bail!("`cargo metadata` failed with {}", out.status);
    }
    serde_json::from_slice(&out.stdout).context("failed to parse the cargo metadata output")
}

fn host_triple() -> Option<String> {
    let out = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
        .arg("-vV")
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.strip_prefix("host: "))
        .map(|t| t.trim().to_string())
}

fn build(args: &Args, meta: Metadata) -> Result<Graph> {
    let mut index = HashMap::new();
    let mut pkgs = Vec::with_capacity(meta.packages.len());

    for (i, p) in meta.packages.iter().enumerate() {
        index.insert(p.id.clone(), i);
        let lib = p.targets.iter().find(|t| {
            t.kind.iter().any(|k| {
                matches!(
                    k.as_str(),
                    "lib" | "rlib" | "dylib" | "cdylib" | "staticlib" | "proc-macro"
                )
            })
        });
        pkgs.push(Pkg {
            name: p.name.clone(),
            version: p.version.clone(),
            lib_name: lib.map(|t| t.name.replace('-', "_")),
            is_proc_macro: p
                .targets
                .iter()
                .any(|t| t.kind.iter().any(|k| k == "proc-macro")),
        });
    }

    let root_id = pick_root(args, &meta)?;
    let root = *index
        .get(&root_id)
        .with_context(|| format!("package `{root_id}` is missing from the metadata"))?;

    let mut edges = vec![Vec::new(); pkgs.len()];
    for node in &meta.resolve.nodes {
        let Some(&from) = index.get(&node.id) else {
            continue;
        };
        for dep in &node.deps {
            let Some(&to) = index.get(&dep.pkg) else {
                continue;
            };
            if !args.proc_macro && pkgs[to].is_proc_macro {
                continue;
            }
            // `dep_kinds` is empty on very old cargo versions; assume normal.
            let wanted = dep.dep_kinds.is_empty()
                || dep.dep_kinds.iter().any(|k| match k.kind.as_deref() {
                    None | Some("normal") => true,
                    Some("dev") => args.dev,
                    Some("build") => args.build,
                    _ => false,
                });
            if wanted && from != to {
                edges[from].push(to);
            }
        }
        edges[from].sort_unstable();
        edges[from].dedup();
    }

    let mut by_crate_name: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, p) in pkgs.iter().enumerate() {
        let name = p
            .lib_name
            .clone()
            .unwrap_or_else(|| p.name.replace('-', "_"));
        by_crate_name.entry(name).or_default().push(i);
    }

    Ok(Graph {
        pkgs,
        edges,
        root,
        by_crate_name,
    })
}

fn pick_root(args: &Args, meta: &Metadata) -> Result<String> {
    let members: Vec<&RawPackage> = meta
        .packages
        .iter()
        .filter(|p| meta.workspace_members.contains(&p.id))
        .collect();

    if let Some(spec) = &args.package {
        // Accept `name`, `name@version` and a full package id.
        let wanted = spec.split('@').next().unwrap_or(spec);
        let hit = members
            .iter()
            .find(|p| p.id == *spec || p.name == wanted)
            .with_context(|| format!("`{spec}` is not a member of this workspace"))?;
        return Ok(hit.id.clone());
    }
    if let Some(root) = &meta.resolve.root {
        return Ok(root.clone());
    }
    let defaults = if meta.workspace_default_members.is_empty() {
        &meta.workspace_members
    } else {
        &meta.workspace_default_members
    };
    if defaults.len() == 1 {
        return Ok(defaults[0].clone());
    }
    let names: Vec<&str> = members.iter().map(|p| p.name.as_str()).collect();
    bail!(
        "this workspace has several packages, pick one with `-p`: {}",
        names.join(", ")
    )
}
