//! Parsing the human forms of times and sizes that users type.

use anyhow::{Result, bail};

/// "90", "90.5", "1:30", "01:02:03.25", "45s" -> seconds.
pub fn parse_time(s: &str) -> Result<f64> {
    let s = s.trim();
    let body = s.strip_suffix('s').unwrap_or(s);
    let parts: Vec<&str> = body.split(':').collect();
    if parts.len() > 3 || parts.iter().any(|p| p.is_empty()) {
        bail!("invalid time {s:?}; use seconds (90) or [h:]m:s (1:30)");
    }
    let mut secs = 0.0;
    for (i, part) in parts.iter().enumerate() {
        let value: f64 = part.parse().map_err(|_| anyhow::anyhow!("invalid time {s:?}"))?;
        let is_last = i == parts.len() - 1;
        if value < 0.0 || (!is_last && value.fract() != 0.0) || (i > 0 && value >= 60.0) {
            bail!("invalid time {s:?}");
        }
        secs = secs * 60.0 + value;
    }
    Ok(secs)
}

/// "h:mm:ss.mmm" with the hours dropped when zero.
pub fn format_time(secs: f64) -> String {
    let ms = (secs.max(0.0) * 1000.0).round() as u64;
    let (h, m, s, ms) = (ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000);
    if h > 0 { format!("{h}:{m:02}:{s:02}.{ms:03}") } else { format!("{m}:{s:02}.{ms:03}") }
}

/// "10MB", "8.5 MB", "700k", "1G", "500000" -> bytes.
///
/// Units are decimal (1 MB = 1,000,000 bytes). That's slightly smaller than
/// the binary MiB some apps mean, so a "10MB" target fits either way.
pub fn parse_size(s: &str) -> Result<u64> {
    let lower = s.trim().to_lowercase();
    let split = lower.find(|c: char| c.is_alphabetic()).unwrap_or(lower.len());
    let (number, unit) = lower.split_at(split);
    let number: f64 = number
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid size {s:?}; e.g. 10MB, 700KB, 1.5GB"))?;
    let scale = match unit.trim() {
        "" | "b" => 1.0,
        "k" | "kb" => 1e3,
        "m" | "mb" => 1e6,
        "g" | "gb" => 1e9,
        other => bail!("unknown size unit {other:?}; use KB, MB or GB"),
    };
    if number <= 0.0 {
        bail!("size must be positive");
    }
    Ok((number * scale) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_times() {
        assert_eq!(parse_time("90").unwrap(), 90.0);
        assert_eq!(parse_time("90.5").unwrap(), 90.5);
        assert_eq!(parse_time("45s").unwrap(), 45.0);
        assert_eq!(parse_time("1:30").unwrap(), 90.0);
        assert_eq!(parse_time("1:02:03.25").unwrap(), 3723.25);
        for bad in ["", "1:", "1:75", "a", "1:2:3:4", "-5", "1.5:00"] {
            assert!(parse_time(bad).is_err(), "{bad:?} should fail");
        }
    }

    #[test]
    fn formats_times() {
        assert_eq!(format_time(7.0), "0:07.000");
        assert_eq!(format_time(3723.25), "1:02:03.250");
    }

    #[test]
    fn parses_sizes() {
        assert_eq!(parse_size("10MB").unwrap(), 10_000_000);
        assert_eq!(parse_size("8.5 mb").unwrap(), 8_500_000);
        assert_eq!(parse_size("700k").unwrap(), 700_000);
        assert_eq!(parse_size("1G").unwrap(), 1_000_000_000);
        assert_eq!(parse_size("500").unwrap(), 500);
        for bad in ["", "MB", "10XB", "0MB", "-1MB"] {
            assert!(parse_size(bad).is_err(), "{bad:?} should fail");
        }
    }
}
