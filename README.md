# bloat-tree

[![CI](https://github.com/co3moz/cargo-bloat-tree/actions/workflows/ci.yml/badge.svg)](https://github.com/co3moz/cargo-bloat-tree/actions/workflows/ci.yml)

```
$ cargo bloat-tree --release

cargo-bloat-tree v0.1.0   file 1.3MiB   .text 1006.0KiB

Direct dependencies (the ones you declared)
      Total       %   Exclusive       Self  Crate
   365.4KiB   36.3%    365.4KiB         0B  clap v4.6.6
    94.6KiB    9.4%     83.3KiB    76.9KiB  serde_json v1.0.151
    14.0KiB    1.4%     14.0KiB    14.0KiB  anyhow v1.0.104
    11.2KiB    1.1%          0B         0B  serde v1.0.229

Everything else
    11.2KiB    1.1%  shared between deps      reachable from more than one direct dependency
   182.5KiB   18.1%  your own code            the analysed crate itself
   312.1KiB   31.0%  std / toolchain          the Rust standard library
       880B    0.1%  build-time only          not linked into the binary: heck v0.5.0, proc-macro2 v1.0.107
     2.6KiB    0.3%  unattributed             cargo bloat could not name these: [Unknown], [unnamed]
    34.0KiB    3.4%  not measured             padding, sections outside .text

Shared crates (no single dependency is to blame)
    11.2KiB    1.1%  serde_core               via serde, serde_json

Tree
   365.4KiB   36.3%  clap v4.6.6
   365.4KiB   36.3%  └── clap_builder v4.6.6 (self 347.6KiB)
    13.5KiB    1.3%      ├── anstream v1.0.0 (self 6.5KiB)
     3.4KiB    0.3%      │   ├── anstyle-parse v1.0.0
     3.3KiB    0.3%      │   ├── anstyle-wincon v3.0.11 (self 1.2KiB)
     2.1KiB    0.2%      │   │   └── anstyle v1.0.14
     2.1KiB    0.2%      │   ├── anstyle v1.0.14 (*)
       298B    0.0%      │   ├── anstyle-query v1.1.5
         8B    0.0%      │   └── colorchoice v1.0.5
     2.5KiB    0.3%      ├── clap_lex v1.1.0
     2.1KiB    0.2%      ├── anstyle v1.0.14 (*)
     1.8KiB    0.2%      └── strsim v0.11.1
    94.6KiB    9.4%  serde_json v1.0.151 (self 76.9KiB)
    11.2KiB    1.1%  ├── serde_core v1.0.229 ~
     4.6KiB    0.5%  ├── memchr v2.8.3
     1.8KiB    0.2%  └── zmij v1.0.23
    ...
```

`cargo bloat --crates` tells you that `regex_automata` takes 330 KiB. It does not
tell you that you never asked for `regex_automata`, `regex` did.

`cargo bloat-tree` takes the output of [`cargo bloat`][bloat] and joins it with the
resolved dependency graph (`cargo metadata`, the same graph `cargo tree` prints), so
every byte is charged to the dependency **you** wrote in `Cargo.toml`.

`clap` contributes no code of its own, but drags in 365 KiB. `serde` is 11 KiB, all of
it in `serde_core`, which `serde_json` needs anyway, dropping `serde` would save
nothing. That is what the `Exclusive` column is for.

## Install

```bash
cargo install cargo-bloat        # the analyser this tool builds on
cargo install --path .           # this tool
```

## Reading the numbers

| Column | Meaning |
| --- | --- |
| `Total` | Everything reachable from this dependency, shared crates included. Totals of different dependencies may overlap. |
| `Exclusive` | What no other direct dependency reaches, the size that would actually disappear if you removed this dependency. |
| `Self` | The crate's own code, without its dependencies. |
| `~` | This crate is also reachable from another direct dependency; it is listed under *Shared crates* instead of being blamed on one of them. |
| `(*)` | This subtree was already printed above (same convention as `cargo tree`). |

All percentages are of the `.text` section, which is what `cargo bloat` measures.

`--attribution` decides how a shared crate is counted in the third column:

- `exclusive` (default), it belongs to nobody, and is reported separately.
- `split`, its size is divided equally between the dependencies that reach it.
- `all`, every dependency is charged the full size (the column then equals `Total`).

## Usage

```bash
# the common case
cargo bloat-tree --release

# only the 10 heaviest branches, hide noise below 8 KiB
cargo bloat-tree --release -n 10 --min-size 8KiB --depth 3

# who is dragging in this crate?
cargo bloat-tree --release --invert regex-syntax

# analyse once, then look at the result as often as you like
cargo bloat-tree --release --save-bloat-json bloat.json
cargo bloat-tree --bloat-json bloat.json --flat
cargo bloat-tree --bloat-json bloat.json --json | jq '.direct[0]'

# or feed it a result you already have
cargo bloat --release --crates -n 0 --message-format json | cargo bloat-tree --bloat-json -
```

`--release`, `--profile`, `--target`, `--features`, `--all-features`,
`--no-default-features`, `-p`, `--bin`, `--lib`, `--example`, `--test`, `--target-dir`,
`--manifest-path`, `--jobs`, `--split-std`, `--locked`, `--offline` and `--frozen` are
passed through to `cargo bloat` and `cargo metadata`. Run `cargo bloat-tree --help` for
the full list.

## What is in the graph

By default only **normal dependencies** are followed, and proc-macro crates are left
out, because their code is not linked into the binary. `--dev`, `--build` and
`--proc-macro` add those edges back. The graph is also filtered to the target platform
you are building for, so `cfg`-gated dependencies of other platforms do not appear.

## Caveats

- `cargo bloat` attributes machine code to crates by symbol name, so its numbers are
  estimates; this tool inherits that. Sizes it cannot attribute end up in
  *unattributed*.
- Only `.text` is measured. Data, relocations and debug info are not.
- If a crate exists in several versions, `cargo bloat` cannot tell them apart. Their
  size is spread evenly over the versions and a note is printed. `cargo tree -d` shows
  which crates are affected.
- Inlining moves code between crates. A crate with a lot of generics can show up as
  much smaller than it is, while its callers grow.
- On MSVC targets `cargo bloat` sometimes reports mangled fragments such as
  `enum2$<regex_syntax`; those are matched back to the crate they name.

## License

MIT, see [LICENSE](LICENSE).

[bloat]: https://github.com/RazrFalcon/cargo-bloat
