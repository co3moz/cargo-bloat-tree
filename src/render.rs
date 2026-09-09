use std::collections::HashSet;
use std::io::Write;

use anyhow::Result;

use crate::analysis::{Analysis, Bucket};
use crate::cli::{Args, Attribution, Charset};
use crate::metadata::Graph;
use crate::size::{human, percent};

pub struct Theme {
    pub color: bool,
    pub charset: Charset,
}

impl Theme {
    fn paint(&self, code: &str, text: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }
    fn bold(&self, text: &str) -> String {
        self.paint("1", text)
    }
    fn dim(&self, text: &str) -> String {
        self.paint("2", text)
    }
    fn cyan(&self, text: &str) -> String {
        self.paint("36", text)
    }
    /// Bigger contributions get louder colours.
    fn size(&self, bytes: u64, total: u64) -> String {
        let text = human(bytes);
        if !self.color {
            return text;
        }
        let share = percent(bytes, total);
        let code = if share >= 10.0 {
            "1;31"
        } else if share >= 2.0 {
            "33"
        } else if bytes == 0 {
            "2"
        } else {
            "0"
        };
        self.paint(code, &text)
    }
    fn glyphs(&self) -> (&'static str, &'static str, &'static str, &'static str) {
        match self.charset {
            // branch, last branch, vertical, blank
            Charset::Utf8 => (
                "\u{251c}\u{2500}\u{2500} ",
                "\u{2514}\u{2500}\u{2500} ",
                "\u{2502}   ",
                "    ",
            ),
            Charset::Ascii => ("|-- ", "`-- ", "|   ", "    "),
        }
    }
}

/// Pads a coloured string to `width` visible characters.
fn pad(text: &str, width: usize) -> String {
    let visible = visible_len(text);
    if visible >= width {
        text.to_string()
    } else {
        format!("{}{}", " ".repeat(width - visible), text)
    }
}

fn pad_right(text: &str, width: usize) -> String {
    let visible = visible_len(text);
    if visible >= width {
        text.to_string()
    } else {
        format!("{}{}", text, " ".repeat(width - visible))
    }
}

fn visible_len(text: &str) -> usize {
    let mut len = 0;
    let mut in_escape = false;
    for c in text.chars() {
        if in_escape {
            in_escape = c != 'm';
        } else if c == '\x1b' {
            in_escape = true;
        } else {
            len += 1;
        }
    }
    len
}

pub fn human_report(
    out: &mut impl Write,
    args: &Args,
    graph: &Graph,
    an: &Analysis,
    theme: &Theme,
    min_size: u64,
) -> Result<()> {
    let root = &graph.pkgs[graph.root];
    writeln!(
        out,
        "\n{} {}   file {}   .text {}",
        theme.bold(&root.name),
        theme.dim(&format!("v{}", root.version)),
        human(an.file_size),
        human(an.text_size),
    )?;

    if let Some(needle) = &args.invert {
        return invert_report(out, graph, an, theme, needle);
    }
    if !args.tree {
        summary(out, args, graph, an, theme, min_size)?;
    }
    if !args.flat {
        tree(out, args, graph, an, theme, min_size)?;
    }
    for note in &an.notes {
        writeln!(out, "\n{} {}", theme.cyan("note:"), note)?;
    }
    Ok(())
}

fn summary(
    out: &mut impl Write,
    args: &Args,
    graph: &Graph,
    an: &Analysis,
    theme: &Theme,
    min_size: u64,
) -> Result<()> {
    let total = an.text_size;
    writeln!(
        out,
        "\n{}",
        theme.bold("Direct dependencies (the ones you declared)")
    )?;
    writeln!(
        out,
        "{}",
        theme.dim(&format!(
            "{}{}{}{}{}",
            pad("Total", 11),
            pad("%", 8),
            pad(args.attribution.column(), 12),
            pad("Self", 11),
            "  Crate"
        ))
    )?;

    let mut shown = 0;
    let mut hidden_total = 0u64;
    let mut hidden_credited = 0u64;
    let mut hidden_count = 0usize;
    for dep in &an.direct {
        let over_limit = args.top > 0 && shown >= args.top;
        if dep.total < min_size || over_limit {
            hidden_total += dep.total;
            hidden_credited += dep.credited;
            hidden_count += 1;
            continue;
        }
        shown += 1;
        let pkg = &graph.pkgs[dep.idx];
        writeln!(
            out,
            "{}{}{}{}  {} {}",
            pad(&theme.size(dep.total, total), 11),
            pad(&format!("{:.1}%", percent(dep.total, total)), 8),
            pad(&human(dep.credited), 12),
            pad(&human(dep.own), 11),
            pkg.name,
            theme.dim(&format!("v{}", pkg.version)),
        )?;
    }
    if hidden_count > 0 {
        writeln!(
            out,
            "{}",
            theme.dim(&format!(
                "{}{}{}{}  {} more direct dependencies",
                pad(&human(hidden_total), 11),
                pad(&format!("{:.1}%", percent(hidden_total, total)), 8),
                pad(&human(hidden_credited), 12),
                pad("", 11),
                hidden_count
            ))
        )?;
    }

    writeln!(out, "\n{}", theme.bold("Everything else"))?;
    let names_in = |bucket: Bucket| -> String {
        let names: Vec<&str> = an
            .unattributed
            .iter()
            .filter(|u| u.bucket == bucket)
            .map(|u| u.name.as_str())
            .collect();
        let head = names.iter().take(3).copied().collect::<Vec<_>>().join(", ");
        if names.len() > 3 {
            format!("{head}, +{} more", names.len() - 3)
        } else {
            head
        }
    };
    let mut row = |label: &str, size: u64, hint: &str| -> Result<()> {
        if size == 0 {
            return Ok(());
        }
        writeln!(
            out,
            "{}{}  {} {}",
            pad(&human(size), 11),
            pad(&format!("{:.1}%", percent(size, total)), 8),
            pad_right(label, 24),
            theme.dim(hint)
        )?;
        Ok(())
    };
    if args.attribution == Attribution::Exclusive {
        row(
            "shared between deps",
            an.shared_total(),
            "reachable from more than one direct dependency",
        )?;
    }
    row("your own code", an.own, "the analysed crate itself")?;
    let std_names = names_in(Bucket::Std);
    row(
        "std / toolchain",
        an.bucket_total(Bucket::Std),
        if std_names == "std" {
            "the Rust standard library"
        } else {
            &std_names
        },
    )?;
    row(
        "build-time only",
        an.bucket_total(Bucket::Outside),
        &format!("not linked into the binary: {}", names_in(Bucket::Outside)),
    )?;
    row(
        "unattributed",
        an.bucket_total(Bucket::Unknown),
        &format!(
            "cargo bloat could not name these: {}",
            names_in(Bucket::Unknown)
        ),
    )?;
    let unmeasured = an.text_size.saturating_sub(an.measured);
    row(
        "not measured",
        unmeasured,
        "padding, sections outside .text",
    )?;

    if !an.shared.is_empty() && args.attribution == Attribution::Exclusive {
        writeln!(
            out,
            "\n{}",
            theme.bold("Shared crates (no single dependency is to blame)")
        )?;
        for shared in an.shared.iter().take(10) {
            if shared.size < min_size {
                continue;
            }
            let owners: Vec<&str> = shared
                .owners
                .iter()
                .map(|&o| graph.pkgs[o].name.as_str())
                .collect();
            writeln!(
                out,
                "{}{}  {} {}",
                pad(&human(shared.size), 11),
                pad(&format!("{:.1}%", percent(shared.size, total)), 8),
                pad_right(&graph.pkgs[shared.idx].name, 24),
                theme.dim(&format!("via {}", owners.join(", "))),
            )?;
        }
    }
    Ok(())
}

struct TreeCtx<'a> {
    graph: &'a Graph,
    an: &'a Analysis,
    theme: &'a Theme,
    min_size: u64,
    max_depth: usize,
    show_empty: bool,
}

fn tree(
    out: &mut impl Write,
    args: &Args,
    graph: &Graph,
    an: &Analysis,
    theme: &Theme,
    min_size: u64,
) -> Result<()> {
    writeln!(out, "\n{}", theme.bold("Tree"))?;
    let ctx = TreeCtx {
        graph,
        an,
        theme,
        min_size,
        max_depth: args.depth.unwrap_or(usize::MAX),
        show_empty: args.show_empty,
    };

    let mut shown = 0;
    for dep in &an.direct {
        if args.top > 0 && shown >= args.top {
            break;
        }
        if dep.total < min_size && !args.show_empty {
            continue;
        }
        shown += 1;
        let mut visited = HashSet::new();
        visited.insert(dep.idx);
        line(out, &ctx, dep.idx, "", true, true)?;
        children(out, &ctx, dep.idx, String::new(), 1, &mut visited)?;
    }

    writeln!(
        out,
        "\n{}",
        theme.dim(&format!(
            "size = whole subtree, (self ...) = the crate's own code, {} = also reachable \
             from another direct dependency, (*) = subtree already shown",
            marker(theme)
        ))
    )?;
    Ok(())
}

fn marker(theme: &Theme) -> String {
    theme.cyan("~")
}

fn children(
    out: &mut impl Write,
    ctx: &TreeCtx<'_>,
    node: usize,
    prefix: String,
    depth: usize,
    visited: &mut HashSet<usize>,
) -> Result<()> {
    if depth >= ctx.max_depth {
        return Ok(());
    }
    let (branch, last_branch, vertical, blank) = ctx.theme.glyphs();

    let mut kids: Vec<usize> = ctx.graph.edges[node]
        .iter()
        .copied()
        .filter(|&c| ctx.show_empty || (ctx.an.subtree[c] > 0 && ctx.an.subtree[c] >= ctx.min_size))
        .collect();
    kids.sort_by(|&a, &b| {
        ctx.an.subtree[b]
            .cmp(&ctx.an.subtree[a])
            .then_with(|| ctx.graph.pkgs[a].name.cmp(&ctx.graph.pkgs[b].name))
    });

    for (i, &kid) in kids.iter().enumerate() {
        let is_last = i + 1 == kids.len();
        let glyph = if is_last { last_branch } else { branch };
        let repeated = !visited.insert(kid);
        line(out, ctx, kid, &format!("{prefix}{glyph}"), false, !repeated)?;
        if !repeated {
            let next = format!("{prefix}{}", if is_last { blank } else { vertical });
            children(out, ctx, kid, next, depth + 1, visited)?;
        }
    }
    Ok(())
}

fn line(
    out: &mut impl Write,
    ctx: &TreeCtx<'_>,
    node: usize,
    prefix: &str,
    is_direct: bool,
    expanded: bool,
) -> Result<()> {
    let theme = ctx.theme;
    let pkg = &ctx.graph.pkgs[node];
    let total = ctx.an.subtree[node];
    let own = ctx.an.sizes[node];

    let name = if is_direct {
        theme.bold(&pkg.name)
    } else {
        pkg.name.clone()
    };

    let mut suffix = String::new();
    if own != total && own > 0 {
        suffix.push_str(&theme.dim(&format!(" (self {})", human(own))));
    }
    if ctx.an.owner_count[node] > 1 {
        suffix.push(' ');
        suffix.push_str(&marker(theme));
    }
    if !expanded {
        suffix.push_str(&theme.dim(" (*)"));
    }

    writeln!(
        out,
        "{}{}  {}{} {}{}",
        pad(&theme.size(total, ctx.an.text_size), 11),
        pad(&format!("{:.1}%", percent(total, ctx.an.text_size)), 8),
        prefix,
        name,
        theme.dim(&format!("v{}", pkg.version)),
        suffix,
    )?;
    Ok(())
}

pub fn json_report(out: &mut impl Write, graph: &Graph, an: &Analysis) -> Result<()> {
    use serde_json::{json, Value};

    let dep_json = |idx: usize| -> Value {
        json!({
            "name": graph.pkgs[idx].name,
            "version": graph.pkgs[idx].version,
        })
    };

    let direct: Vec<Value> = an
        .direct
        .iter()
        .map(|d| {
            json!({
                "name": graph.pkgs[d.idx].name,
                "version": graph.pkgs[d.idx].version,
                "total": d.total,
                "exclusive": d.exclusive,
                "self": d.own,
                "credited": d.credited,
                "percent": percent(d.total, an.text_size),
            })
        })
        .collect();

    let shared: Vec<Value> = an
        .shared
        .iter()
        .map(|s| {
            json!({
                "name": graph.pkgs[s.idx].name,
                "version": graph.pkgs[s.idx].version,
                "size": s.size,
                "via": s.owners.iter().map(|&o| dep_json(o)).collect::<Vec<_>>(),
            })
        })
        .collect();

    let unattributed: Vec<Value> = an
        .unattributed
        .iter()
        .map(|u| {
            json!({
                "name": u.name,
                "size": u.size,
                "bucket": match u.bucket {
                    Bucket::Std => "std",
                    Bucket::Outside => "build-time",
                    Bucket::Unknown => "unknown",
                },
            })
        })
        .collect();

    let root = graph.root;
    let value = json!({
        "package": graph.pkgs[root].name,
        "version": graph.pkgs[root].version,
        "file-size": an.file_size,
        "text-section-size": an.text_size,
        "measured": an.measured,
        "own": an.own,
        "direct": direct,
        "shared": shared,
        "unattributed": unattributed,
        "notes": an.notes,
    });
    writeln!(out, "{}", serde_json::to_string_pretty(&value)?)?;
    Ok(())
}

/// `cargo tree -i` for a single crate: every path that drags it in.
pub fn invert_report(
    out: &mut impl Write,
    graph: &Graph,
    an: &Analysis,
    theme: &Theme,
    needle: &str,
) -> Result<()> {
    let key = needle.replace('-', "_");
    let targets: Vec<usize> = (0..graph.pkgs.len())
        .filter(|&i| {
            let pkg = &graph.pkgs[i];
            pkg.name.replace('-', "_") == key || pkg.lib_name.as_deref() == Some(key.as_str())
        })
        .collect();
    if targets.is_empty() {
        anyhow::bail!("`{needle}` is not in the dependency graph of this package (a proc-macro or build dependency? try --proc-macro / --build)");
    }

    let mut rev: Vec<Vec<usize>> = vec![Vec::new(); graph.pkgs.len()];
    for (from, deps) in graph.edges.iter().enumerate() {
        for &to in deps {
            rev[to].push(from);
        }
    }

    for &target in &targets {
        let pkg = &graph.pkgs[target];
        writeln!(
            out,
            "
{} {}   own {}   subtree {}",
            theme.bold(&pkg.name),
            theme.dim(&format!("v{}", pkg.version)),
            theme.size(an.sizes[target], an.text_size),
            human(an.subtree[target]),
        )?;
        if rev[target].is_empty() {
            writeln!(out, "{}", theme.dim("  nothing depends on it"))?;
            continue;
        }
        let mut path = vec![target];
        let mut seen = HashSet::new();
        seen.insert(target);
        let ctx = InvertCtx {
            graph,
            an,
            theme,
            rev: &rev,
        };
        invert_children(out, &ctx, target, String::new(), &mut path, &mut seen)?;
    }
    writeln!(
        out,
        "
{}",
        theme.dim(
            "size = the crate's own code, [direct] = declared in your Cargo.toml, (*) = already shown above",
        )
    )?;
    Ok(())
}

struct InvertCtx<'a> {
    graph: &'a Graph,
    an: &'a Analysis,
    theme: &'a Theme,
    rev: &'a [Vec<usize>],
}

fn invert_children(
    out: &mut impl Write,
    ctx: &InvertCtx<'_>,
    node: usize,
    prefix: String,
    path: &mut Vec<usize>,
    seen: &mut HashSet<usize>,
) -> Result<()> {
    let InvertCtx {
        graph,
        an,
        theme,
        rev,
    } = *ctx;
    let (branch, last_branch, vertical, blank) = theme.glyphs();
    let mut parents: Vec<usize> = rev[node]
        .iter()
        .copied()
        .filter(|p| !path.contains(p))
        .collect();
    parents.sort_by(|&a, &b| graph.pkgs[a].name.cmp(&graph.pkgs[b].name));

    for (i, &parent) in parents.iter().enumerate() {
        let is_last = i + 1 == parents.len();
        let glyph = if is_last { last_branch } else { branch };
        let pkg = &graph.pkgs[parent];
        let direct = graph.edges[graph.root].contains(&parent);
        let repeated = !seen.insert(parent);
        let tag = if parent == graph.root {
            theme.dim(" (this package)")
        } else if direct {
            format!(" {}", theme.cyan("[direct]"))
        } else {
            String::new()
        };
        writeln!(
            out,
            "{}{}  {}{}{} {}{}",
            pad(&theme.size(an.sizes[parent], an.text_size), 11),
            pad(
                &format!("{:.1}%", percent(an.sizes[parent], an.text_size)),
                8
            ),
            prefix,
            glyph,
            if direct {
                theme.bold(&pkg.name)
            } else {
                pkg.name.clone()
            },
            theme.dim(&format!("v{}", pkg.version)),
            if repeated {
                format!("{tag}{}", theme.dim(" (*)"))
            } else {
                tag
            },
        )?;
        if repeated {
            continue;
        }
        path.push(parent);
        let next = format!("{prefix}{}", if is_last { blank } else { vertical });
        invert_children(out, ctx, parent, next, path, seen)?;
        path.pop();
    }
    Ok(())
}
