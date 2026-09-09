use clap::{Parser, ValueEnum};

/// `cargo bloat-tree`, map `cargo bloat --crates` sizes onto the dependency tree,
/// so you can see *which of your own dependencies* drags in the bloat.
#[derive(Parser, Debug)]
#[command(
    name = "cargo-bloat-tree",
    bin_name = "cargo bloat-tree",
    version,
    about = "Cargo dependency bloat mapper",
    after_help = "\
EXAMPLES:
    cargo bloat-tree --release
    cargo bloat-tree --release --bin my-app --min-size 8KiB
    cargo bloat-tree --release --save-bloat-json bloat.json   # analyse once, ...
    cargo bloat-tree --bloat-json bloat.json --flat           # ... re-view for free
    cargo bloat --release --crates -n 0 --message-format json | cargo bloat-tree --bloat-json -"
)]
pub struct Args {
    // ------------------------------------------------------------------ input
    /// Reuse an existing `cargo bloat --crates --message-format json` result
    /// instead of building ("-" reads stdin)
    #[arg(long, value_name = "PATH", help_heading = "Input")]
    pub bloat_json: Option<String>,

    /// Install cargo-bloat without asking if it is missing
    #[arg(long, help_heading = "Input")]
    pub install_cargo_bloat: bool,

    /// Write the `cargo bloat` JSON that was produced to this file
    #[arg(long, value_name = "PATH", help_heading = "Input")]
    pub save_bloat_json: Option<String>,

    // ------------------------------------------------------------------ build
    /// Build artifacts in release mode, with optimizations
    #[arg(long, help_heading = "Build")]
    pub release: bool,

    /// Build with the given profile
    #[arg(long, value_name = "PROFILE", help_heading = "Build")]
    pub profile: Option<String>,

    /// Build for the target triple
    #[arg(long, value_name = "TRIPLE", help_heading = "Build")]
    pub target: Option<String>,

    /// Space-separated list of features to activate
    #[arg(long, value_name = "FEATURES", help_heading = "Build")]
    pub features: Option<String>,

    /// Activate all available features
    #[arg(long, help_heading = "Build")]
    pub all_features: bool,

    /// Do not activate the `default` feature
    #[arg(long, help_heading = "Build")]
    pub no_default_features: bool,

    /// Package to analyse (required in a workspace with several members)
    #[arg(short = 'p', long, value_name = "SPEC", help_heading = "Build")]
    pub package: Option<String>,

    /// Analyse only this package's library
    #[arg(long, help_heading = "Build")]
    pub lib: bool,

    /// Analyse only the specified binary
    #[arg(long, value_name = "NAME", help_heading = "Build")]
    pub bin: Option<String>,

    /// Analyse only the specified example
    #[arg(long, value_name = "NAME", help_heading = "Build")]
    pub example: Option<String>,

    /// Analyse only the specified test target
    #[arg(long, value_name = "NAME", help_heading = "Build")]
    pub test: Option<String>,

    /// Directory for all generated artifacts
    #[arg(long, value_name = "DIRECTORY", help_heading = "Build")]
    pub target_dir: Option<String>,

    /// Path to Cargo.toml
    #[arg(long, value_name = "PATH", help_heading = "Build")]
    pub manifest_path: Option<String>,

    /// Number of parallel jobs, defaults to # of CPUs
    #[arg(short = 'j', long, value_name = "N", help_heading = "Build")]
    pub jobs: Option<String>,

    /// Split the 'std' crate into core, alloc, etc.
    #[arg(long, help_heading = "Build")]
    pub split_std: bool,

    /// Require Cargo.lock is up to date
    #[arg(long, help_heading = "Build")]
    pub locked: bool,

    /// Run without accessing the network
    #[arg(long, help_heading = "Build")]
    pub offline: bool,

    /// Require Cargo.lock and cache are up to date
    #[arg(long, help_heading = "Build")]
    pub frozen: bool,

    // ------------------------------------------------------------------ graph
    /// Also follow dev-dependencies
    #[arg(long, help_heading = "Dependency graph")]
    pub dev: bool,

    /// Also follow build-dependencies
    #[arg(long, help_heading = "Dependency graph")]
    pub build: bool,

    /// Also follow proc-macro crates (their code is not linked into the binary)
    #[arg(long = "proc-macro", help_heading = "Dependency graph")]
    pub proc_macro: bool,

    // ------------------------------------------------------------------ output
    /// Show every path that pulls in this crate, like `cargo tree -i`
    #[arg(short = 'i', long, value_name = "CRATE", help_heading = "Output")]
    pub invert: Option<String>,

    /// Show only the N heaviest direct dependencies (0 = all)
    #[arg(
        short = 'n',
        long,
        value_name = "NUM",
        default_value_t = 0,
        help_heading = "Output"
    )]
    pub top: usize,

    /// Maximum depth of the printed tree
    #[arg(short = 'd', long, value_name = "DEPTH", help_heading = "Output")]
    pub depth: Option<usize>,

    /// Hide crates smaller than this (e.g. 4KiB, 100kb, 2MiB)
    #[arg(
        long,
        value_name = "SIZE",
        default_value = "0",
        help_heading = "Output"
    )]
    pub min_size: String,

    /// How to count a crate that several direct dependencies pull in
    #[arg(long, value_enum, default_value_t = Attribution::Exclusive, help_heading = "Output")]
    pub attribution: Attribution,

    /// Sort direct dependencies by this column
    #[arg(long, value_enum, default_value_t = Sort::Total, help_heading = "Output")]
    pub sort: Sort,

    /// Print a flat table only, without the tree
    #[arg(long, help_heading = "Output")]
    pub flat: bool,

    /// Print the tree only, without the summary table
    #[arg(long, conflicts_with = "flat", help_heading = "Output")]
    pub tree: bool,

    /// Machine readable output
    #[arg(long, conflicts_with_all = ["flat", "tree"], help_heading = "Output")]
    pub json: bool,

    /// Keep branches that contribute no measurable size
    #[arg(long, help_heading = "Output")]
    pub show_empty: bool,

    /// Character set for the tree
    #[arg(long, value_enum, default_value_t = Charset::Utf8, help_heading = "Output")]
    pub charset: Charset,

    /// Coloring
    #[arg(long, value_enum, default_value_t = Color::Auto, help_heading = "Output")]
    pub color: Color,

    /// Print the commands that are being run
    #[arg(short = 'v', long, help_heading = "Output")]
    pub verbose: bool,
}

impl Attribution {
    pub fn column(self) -> &'static str {
        match self {
            Attribution::Exclusive => "Exclusive",
            Attribution::Split => "Split",
            Attribution::All => "Inclusive",
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum Attribution {
    /// A crate reachable from several direct dependencies is listed separately as "shared"
    Exclusive,
    /// Its size is divided equally between the direct dependencies that reach it
    Split,
    /// Its full size is counted for every direct dependency that reaches it (totals overlap)
    All,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum Sort {
    /// Everything below this dependency, shared crates included
    Total,
    /// Only what would actually go away if this dependency was dropped
    Exclusive,
    /// The crate's own code only
    SelfSize,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum Charset {
    Utf8,
    Ascii,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum Color {
    Auto,
    Always,
    Never,
}

impl Args {
    /// Handles being invoked as `cargo bloat-tree`, where cargo passes the
    /// subcommand name as the first argument.
    pub fn from_env() -> Args {
        let mut argv: Vec<std::ffi::OsString> = std::env::args_os().collect();
        if argv.get(1).map(|a| a == "bloat-tree").unwrap_or(false) {
            argv.remove(1);
        }
        Args::parse_from(argv)
    }

    pub fn use_color(&self) -> bool {
        match self.color {
            Color::Always => true,
            Color::Never => false,
            Color::Auto => {
                use std::io::IsTerminal;
                std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal()
            }
        }
    }
}
