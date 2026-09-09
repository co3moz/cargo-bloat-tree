mod analysis;
mod bloat;
mod cli;
mod metadata;
mod render;
mod size;

use std::io::Write;

use anyhow::Result;

use cli::{Args, Charset};
use render::Theme;

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = Args::from_env();
    let min_size = size::parse(&args.min_size)?;

    let bloat = bloat::obtain(&args)?;
    let graph = metadata::load(&args)?;
    let analysis = analysis::analyse(&args, &graph, &bloat);

    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());

    if args.json {
        render::json_report(&mut out, &graph, &analysis)?;
    } else {
        let theme = Theme {
            color: args.use_color(),
            charset: match args.charset {
                Charset::Utf8 => Charset::Utf8,
                Charset::Ascii => Charset::Ascii,
            },
        };
        render::human_report(&mut out, &args, &graph, &analysis, &theme, min_size)?;
    }
    out.flush()?;
    Ok(())
}
