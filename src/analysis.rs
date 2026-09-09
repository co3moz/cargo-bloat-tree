use crate::bloat::BloatOutput;
use crate::cli::{Args, Attribution, Sort};
use crate::metadata::Graph;

/// Crates that `--split-std` pulls out of `std`. They are only treated as
/// part of the toolchain when no package of that name exists in the graph.
const STD_CRATES: &[&str] = &[
    "std",
    "core",
    "alloc",
    "proc_macro",
    "test",
    "unwind",
    "panic_abort",
    "panic_unwind",
    "compiler_builtins",
    "rustc_demangle",
    "rustc_std_workspace_core",
    "rustc_std_workspace_alloc",
    "rustc_std_workspace_std",
    "std_detect",
    "addr2line",
    "gimli",
    "object",
    "miniz_oxide",
    "adler",
    "adler2",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bucket {
    /// Rust standard library / toolchain code.
    Std,
    /// A package that exists, but is not reachable through the selected edges
    /// (a proc-macro, a build dependency, ...).
    Outside,
    /// A symbol `cargo bloat` could not attribute to any crate.
    Unknown,
}

#[derive(Debug)]
pub struct Unattributed {
    pub name: String,
    pub size: u64,
    pub bucket: Bucket,
}

#[derive(Debug)]
pub struct DirectDep {
    pub idx: usize,
    /// Everything reachable from here, shared crates included.
    pub total: u64,
    /// What no other direct dependency reaches, i.e. the size that would
    /// actually disappear if this dependency was dropped.
    pub exclusive: u64,
    /// The crate's own code.
    pub own: u64,
    /// The value this dependency is credited with under the chosen attribution.
    pub credited: u64,
}

#[derive(Debug)]
pub struct SharedCrate {
    pub idx: usize,
    pub size: u64,
    /// Direct dependencies that reach it (indices into `Graph::pkgs`).
    pub owners: Vec<usize>,
}

pub struct Analysis {
    pub file_size: u64,
    pub text_size: u64,
    /// Sum of every crate size reported by `cargo bloat`.
    pub measured: u64,
    /// Per package size, indexed like `Graph::pkgs`.
    pub sizes: Vec<u64>,
    /// Size of everything reachable from a package, itself included.
    pub subtree: Vec<u64>,
    /// How many direct dependencies reach a package.
    pub owner_count: Vec<u32>,
    pub direct: Vec<DirectDep>,
    /// Code of the analysed package itself.
    pub own: u64,
    pub shared: Vec<SharedCrate>,
    pub unattributed: Vec<Unattributed>,
    pub notes: Vec<String>,
}

impl Analysis {
    pub fn shared_total(&self) -> u64 {
        self.shared.iter().map(|s| s.size).sum()
    }

    pub fn bucket_total(&self, bucket: Bucket) -> u64 {
        self.unattributed
            .iter()
            .filter(|u| u.bucket == bucket)
            .map(|u| u.size)
            .sum()
    }
}

pub fn analyse(args: &Args, graph: &Graph, bloat: &BloatOutput) -> Analysis {
    let n = graph.pkgs.len();
    let reach = Reach::compute(&graph.edges);

    let mut sizes = vec![0u64; n];
    let mut unattributed: Vec<Unattributed> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let mut measured = 0u64;
    let mut multi_version: Vec<String> = Vec::new();

    for entry in &bloat.crates {
        measured += entry.size;
        match resolve_crate(graph, &entry.name) {
            Resolved::Pkgs(idxs) => {
                let live: Vec<usize> = idxs
                    .iter()
                    .copied()
                    .filter(|&i| reach.contains(graph.root, i))
                    .collect();
                if live.is_empty() {
                    let label = graph.pkgs[idxs[0]].label();
                    push_unattributed(&mut unattributed, label, entry.size, Bucket::Outside);
                } else {
                    if live.len() > 1 && !multi_version.contains(&entry.name) {
                        multi_version.push(entry.name.clone());
                    }
                    // Several versions of one crate are indistinguishable in
                    // the bloat output, so the size is spread over them.
                    let share = entry.size / live.len() as u64;
                    let mut rest = entry.size - share * live.len() as u64;
                    for &i in &live {
                        sizes[i] += share + rest.min(1);
                        rest = rest.saturating_sub(1);
                    }
                }
            }
            Resolved::Std => {
                push_unattributed(&mut unattributed, entry.name.clone(), entry.size, Bucket::Std)
            }
            Resolved::Unknown => {
                let name = if entry.name.trim().is_empty() {
                    "[unnamed]".to_string()
                } else {
                    entry.name.clone()
                };
                push_unattributed(&mut unattributed, name, entry.size, Bucket::Unknown)
            }
        }
    }

    if !multi_version.is_empty() {
        notes.push(format!(
            "{} exists in more than one version; cargo bloat cannot tell the versions apart, \
             so the size was spread evenly over them (see `cargo tree -d`)",
            multi_version.join(", ")
        ));
    }

    let subtree: Vec<u64> = (0..n)
        .map(|i| reach.iter(i).map(|j| sizes[j]).sum())
        .collect();

    let directs: Vec<usize> = graph.edges[graph.root].clone();

    let mut owner_count = vec![0u32; n];
    for &d in &directs {
        for j in reach.iter(d) {
            owner_count[j] += 1;
        }
    }

    let mut direct: Vec<DirectDep> = directs
        .iter()
        .map(|&d| {
            let mut total = 0;
            let mut exclusive = 0;
            let mut credited = 0;
            for j in reach.iter(d) {
                let s = sizes[j];
                total += s;
                if owner_count[j] == 1 {
                    exclusive += s;
                }
                credited += match args.attribution {
                    Attribution::Exclusive => {
                        if owner_count[j] == 1 {
                            s
                        } else {
                            0
                        }
                    }
                    Attribution::Split => s / owner_count[j].max(1) as u64,
                    Attribution::All => s,
                };
            }
            DirectDep {
                idx: d,
                total,
                exclusive,
                own: sizes[d],
                credited,
            }
        })
        .collect();

    direct.sort_by(|a, b| {
        let key = |x: &DirectDep| match args.sort {
            Sort::Total => x.total,
            Sort::Exclusive => x.exclusive,
            Sort::SelfSize => x.own,
        };
        key(b)
            .cmp(&key(a))
            .then_with(|| graph.pkgs[a.idx].name.cmp(&graph.pkgs[b.idx].name))
    });

    let mut shared: Vec<SharedCrate> = (0..n)
        .filter(|&i| owner_count[i] > 1 && sizes[i] > 0)
        .map(|i| SharedCrate {
            idx: i,
            size: sizes[i],
            owners: directs
                .iter()
                .copied()
                .filter(|&d| reach.contains(d, i))
                .collect(),
        })
        .collect();
    shared.sort_by_key(|s| std::cmp::Reverse(s.size));

    unattributed.sort_by_key(|u| std::cmp::Reverse(u.size));

    Analysis {
        file_size: bloat.file_size,
        text_size: bloat.text_size,
        measured,
        own: sizes[graph.root],
        sizes,
        subtree,
        owner_count,
        direct,
        shared,
        unattributed,
        notes,
    }
}

fn push_unattributed(list: &mut Vec<Unattributed>, name: String, size: u64, bucket: Bucket) {
    match list.iter_mut().find(|u| u.name == name) {
        Some(existing) => existing.size += size,
        None => list.push(Unattributed { name, size, bucket }),
    }
}

enum Resolved {
    Pkgs(Vec<usize>),
    Std,
    Unknown,
}

/// `cargo bloat` reports compiler crate names, and on MSVC targets it also
/// reports mangled fragments such as `enum2$<regex_syntax`.
fn resolve_crate(graph: &Graph, name: &str) -> Resolved {
    if let Some(idxs) = graph.by_crate_name.get(name) {
        return Resolved::Pkgs(idxs.clone());
    }
    if STD_CRATES.contains(&name) {
        return Resolved::Std;
    }
    let mut best: Option<&str> = None;
    for token in name.split(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
        if token.len() < 3 || !graph.by_crate_name.contains_key(token) {
            continue;
        }
        if best.is_none_or(|b| token.len() > b.len()) {
            best = Some(token);
        }
    }
    match best {
        Some(token) => Resolved::Pkgs(graph.by_crate_name[token].clone()),
        None => Resolved::Unknown,
    }
}

/// Transitive reachability over the dependency graph, one bitset row per package.
pub struct Reach {
    stride: usize,
    bits: Vec<u64>,
    n: usize,
}

impl std::fmt::Debug for Reach {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Reach({} packages)", self.n)
    }
}

impl Reach {
    pub fn compute(edges: &[Vec<usize>]) -> Reach {
        let n = edges.len();
        let stride = n.div_ceil(64).max(1);
        let mut bits = vec![0u64; n * stride];
        for i in 0..n {
            bits[i * stride + i / 64] |= 1 << (i % 64);
        }

        let order = postorder(edges);
        let mut tmp = vec![0u64; stride];
        // One pass is enough for a DAG walked in post-order; the loop only
        // runs again when dev-dependency edges introduce a cycle.
        loop {
            let mut changed = false;
            for &i in &order {
                for &c in &edges[i] {
                    tmp.copy_from_slice(&bits[c * stride..(c + 1) * stride]);
                    let row = &mut bits[i * stride..(i + 1) * stride];
                    for (word, add) in row.iter_mut().zip(&tmp) {
                        let merged = *word | *add;
                        if merged != *word {
                            *word = merged;
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
        Reach { stride, bits, n }
    }

    pub fn contains(&self, from: usize, to: usize) -> bool {
        self.bits[from * self.stride + to / 64] & (1 << (to % 64)) != 0
    }

    pub fn iter(&self, from: usize) -> impl Iterator<Item = usize> + '_ {
        let row = &self.bits[from * self.stride..(from + 1) * self.stride];
        let n = self.n;
        row.iter().enumerate().flat_map(move |(w, &word)| {
            (0..64)
                .filter(move |b| word & (1u64 << b) != 0)
                .map(move |b| w * 64 + b)
                .filter(move |&i| i < n)
        })
    }
}

fn postorder(edges: &[Vec<usize>]) -> Vec<usize> {
    let n = edges.len();
    let mut seen = vec![false; n];
    let mut order = Vec::with_capacity(n);
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for start in 0..n {
        if seen[start] {
            continue;
        }
        seen[start] = true;
        stack.push((start, 0));
        while let Some((node, cursor)) = stack.pop() {
            match edges[node].get(cursor) {
                Some(&next) => {
                    stack.push((node, cursor + 1));
                    if !seen[next] {
                        seen[next] = true;
                        stack.push((next, 0));
                    }
                }
                None => order.push(node),
            }
        }
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reachability_follows_edges() {
        // 0 -> 1 -> 3, 0 -> 2 -> 3
        let edges = vec![vec![1, 2], vec![3], vec![3], vec![]];
        let reach = Reach::compute(&edges);
        assert!(reach.contains(0, 3));
        assert!(reach.contains(1, 3));
        assert!(!reach.contains(3, 0));
        assert!(!reach.contains(1, 2));
        assert_eq!(reach.iter(0).collect::<Vec<_>>(), vec![0, 1, 2, 3]);
        assert_eq!(reach.iter(2).collect::<Vec<_>>(), vec![2, 3]);
    }

    fn demo_graph() -> Graph {
        use crate::metadata::Pkg;
        use std::collections::HashMap;

        let names = ["app", "regex", "aho-corasick", "memchr", "serde_json"];
        let pkgs: Vec<Pkg> = names
            .iter()
            .map(|n| Pkg {
                name: n.to_string(),
                version: "1.0.0".into(),
                lib_name: Some(n.replace('-', "_")),
                is_proc_macro: false,
            })
            .collect();
        let mut by_crate_name: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, p) in pkgs.iter().enumerate() {
            by_crate_name.insert(p.lib_name.clone().unwrap(), vec![i]);
        }
        Graph {
            pkgs,
            edges: vec![vec![1, 4], vec![2, 3], vec![3], vec![], vec![3]],
            root: 0,
            by_crate_name,
        }
    }

    fn bloat(pairs: &[(&str, u64)]) -> BloatOutput {
        BloatOutput {
            file_size: 2000,
            text_size: 1000,
            crates: pairs
                .iter()
                .map(|(name, size)| crate::bloat::CrateSize {
                    name: name.to_string(),
                    size: *size,
                })
                .collect(),
        }
    }

    fn args() -> Args {
        use clap::Parser;
        Args::parse_from(["cargo-bloat-tree"])
    }

    #[test]
    fn splits_shared_crates_out_of_the_blame() {
        let graph = demo_graph();
        let out = bloat(&[
            ("regex", 100),
            ("aho_corasick", 200),
            ("memchr", 50),
            ("serde_json", 30),
            ("app", 10),
            ("std", 500),
            ("`__scrt_common_main_seh'", 7),
        ]);
        let an = analyse(&args(), &graph, &out);

        let regex = an.direct.iter().find(|d| d.idx == 1).unwrap();
        assert_eq!(regex.total, 350, "regex + aho-corasick + memchr");
        assert_eq!(regex.exclusive, 300, "memchr is also reachable from serde_json");
        assert_eq!(regex.own, 100);

        let json = an.direct.iter().find(|d| d.idx == 4).unwrap();
        assert_eq!(json.total, 80);
        assert_eq!(json.exclusive, 30);

        assert_eq!(an.own, 10, "the analysed crate itself");
        assert_eq!(an.shared.len(), 1);
        assert_eq!(an.shared[0].idx, 3);
        assert_eq!(an.shared[0].owners, vec![1, 4]);
        assert_eq!(an.bucket_total(Bucket::Std), 500);
        assert_eq!(an.bucket_total(Bucket::Unknown), 7);
        assert_eq!(an.measured, 897);
        // exclusive + shared + own + std + unknown covers everything measured
        assert_eq!(
            regex.exclusive + json.exclusive + an.shared_total() + an.own + 500 + 7,
            an.measured
        );
    }

    #[test]
    fn splits_shared_sizes_when_asked() {
        let graph = demo_graph();
        let out = bloat(&[("memchr", 50), ("regex", 100)]);
        let mut a = args();
        a.attribution = Attribution::Split;
        let an = analyse(&a, &graph, &out);
        let regex = an.direct.iter().find(|d| d.idx == 1).unwrap();
        assert_eq!(regex.credited, 125, "100 of its own plus half of memchr");
    }

    #[test]
    fn recovers_crate_names_from_msvc_debug_symbols() {
        let graph = demo_graph();
        let out = bloat(&[("enum2$<regex_syntax", 10), ("aho_corasick", 5)]);
        let an = analyse(&args(), &graph, &out);
        // `regex_syntax` is not in this graph, so the fragment stays unattributed,
        // while a plain name still resolves.
        assert_eq!(an.bucket_total(Bucket::Unknown), 10);
        assert_eq!(an.sizes[2], 5);

        let out = bloat(&[("enum2$<memchr", 10)]);
        let an = analyse(&args(), &graph, &out);
        assert_eq!(an.sizes[3], 10, "the crate name inside the fragment wins");
    }

    #[test]
    fn reachability_survives_cycles() {
        let edges = vec![vec![1], vec![2], vec![1, 3], vec![]];
        let reach = Reach::compute(&edges);
        assert!(reach.contains(0, 3));
        assert!(reach.contains(1, 1));
        assert!(reach.contains(2, 1));
    }
}
