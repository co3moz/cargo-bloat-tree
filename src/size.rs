use anyhow::{bail, Result};

/// Human readable byte size, in the same units `cargo bloat` uses.
pub fn human(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= MIB {
        format!("{:.1}MiB", b / MIB)
    } else if b >= KIB {
        format!("{:.1}KiB", b / KIB)
    } else {
        format!("{}B", bytes)
    }
}

pub fn percent(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 * 100.0 / whole as f64
    }
}

/// Parses `0`, `512`, `4KiB`, `100kb`, `2MiB`, `1.5mb`.
pub fn parse(input: &str) -> Result<u64> {
    let s = input.trim();
    let split = s
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let num: f64 = num
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid size `{input}`"))?;
    let mult = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1.0,
        "k" | "kb" | "kib" => 1024.0,
        "m" | "mb" | "mib" => 1024.0 * 1024.0,
        "g" | "gb" | "gib" => 1024.0 * 1024.0 * 1024.0,
        other => bail!("unknown size unit `{other}` in `{input}`"),
    };
    Ok((num * mult) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sizes() {
        assert_eq!(parse("0").unwrap(), 0);
        assert_eq!(parse("512").unwrap(), 512);
        assert_eq!(parse("4KiB").unwrap(), 4096);
        assert_eq!(parse("4kb").unwrap(), 4096);
        assert_eq!(parse("1.5MiB").unwrap(), 1_572_864);
        assert!(parse("12qb").is_err());
    }

    #[test]
    fn formats_sizes() {
        assert_eq!(human(10), "10B");
        assert_eq!(human(2048), "2.0KiB");
        assert_eq!(human(1024 * 1024 * 3 / 2), "1.5MiB");
    }
}
