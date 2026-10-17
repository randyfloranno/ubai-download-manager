//! Locale-aware number, size, speed and duration formatting.

use crate::locale::Locale;
use udm_core::units::iec_parts;

/// Formats an integer with locale grouping (`1.234.567` in `id`, `1,234,567` in `en`).
pub fn format_number(locale: Locale, value: u64) -> String {
    let digits = value.to_string();
    let len = digits.len();
    let mut out = String::with_capacity(len.saturating_add(len / 3));
    for (i, ch) in digits.chars().enumerate() {
        let remaining = len.saturating_sub(i);
        if i > 0 && remaining % 3 == 0 {
            out.push(locale.group_separator());
        }
        out.push(ch);
    }
    out
}

/// Formats a byte count with IEC units and one decimal (`1,5 MiB` / `1.5 MiB`).
pub fn format_bytes(locale: Locale, bytes: u64) -> String {
    let parts = iec_parts(bytes);
    if parts.unit == "B" {
        return format!("{} B", format_number(locale, parts.whole));
    }
    format!(
        "{}{}{} {}",
        format_number(locale, parts.whole),
        locale.decimal_separator(),
        parts.tenth,
        parts.unit
    )
}

/// Formats a speed in bytes per second (`1,5 MiB/s`).
pub fn format_speed(locale: Locale, bytes_per_sec: u64) -> String {
    format!("{}/s", format_bytes(locale, bytes_per_sec))
}

/// Formats a percentage with one decimal from a numerator/denominator pair.
pub fn format_percent(locale: Locale, done: u64, total: u64) -> String {
    if total == 0 {
        return format!("0{}0%", locale.decimal_separator());
    }
    let permille = u128::from(done)
        .saturating_mul(1000)
        .checked_div(u128::from(total))
        .unwrap_or(0)
        .min(1000);
    let whole = permille / 10;
    let tenth = permille % 10;
    format!("{whole}{}{tenth}%", locale.decimal_separator())
}

/// Formats a duration as `1j 02m 03d` (id) / `1h 02m 03s` (en); minutes and
/// seconds only when under an hour.
pub fn format_duration(locale: Locale, secs: u64) -> String {
    let (h, m, s) = match locale {
        Locale::Id => ("j", "m", "d"),
        Locale::En => ("h", "m", "s"),
    };
    let hours = secs / 3600;
    let minutes = (secs % 3600) / 60;
    let seconds = secs % 60;
    if hours > 0 {
        format!(
            "{}{h} {minutes:02}{m} {seconds:02}{s}",
            format_number(locale, hours)
        )
    } else if minutes > 0 {
        format!("{minutes}{m} {seconds:02}{s}")
    } else {
        format!("{seconds}{s}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_speed_id_uses_comma() {
        assert_eq!(format_speed(Locale::Id, 1_572_864), "1,5 MiB/s");
        assert_eq!(format_speed(Locale::En, 1_572_864), "1.5 MiB/s");
    }

    #[test]
    fn numbers_grouped() {
        assert_eq!(format_number(Locale::Id, 1_234_567), "1.234.567");
        assert_eq!(format_number(Locale::En, 1_234_567), "1,234,567");
        assert_eq!(format_number(Locale::En, 12), "12");
        assert_eq!(format_bytes(Locale::Id, 999), "999 B");
        assert_eq!(format_bytes(Locale::En, u64::MAX), "16,777,215.9 TiB");
    }

    #[test]
    fn percent_and_duration() {
        assert_eq!(format_percent(Locale::Id, 1, 3), "33,3%");
        assert_eq!(format_percent(Locale::En, 5, 0), "0.0%");
        assert_eq!(format_percent(Locale::En, 9, 3), "100.0%");
        assert_eq!(format_duration(Locale::Id, 3_723), "1j 02m 03d");
        assert_eq!(format_duration(Locale::En, 65), "1m 05s");
        assert_eq!(format_duration(Locale::En, 7), "7s");
    }
}
