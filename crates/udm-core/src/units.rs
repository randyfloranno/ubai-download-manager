//! Byte units. Sizes are `u64` bytes; display uses IEC units (KiB, MiB, ...).

/// 1 KiB.
pub const KIB: u64 = 1_024;
/// 1 MiB.
pub const MIB: u64 = 1_048_576;
/// 1 GiB.
pub const GIB: u64 = 1_073_741_824;
/// 1 TiB.
pub const TIB: u64 = 1_099_511_627_776;

/// IEC unit labels, index = power of 1024.
pub const IEC_UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];

/// A size split for display: `whole.tenth unit`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IecParts {
    /// Integer part.
    pub whole: u64,
    /// First decimal digit (0–9). Always 0 for bytes.
    pub tenth: u8,
    /// Unit label.
    pub unit: &'static str,
}

/// Splits a byte count into IEC display parts with one (truncated) decimal.
pub fn iec_parts(bytes: u64) -> IecParts {
    let steps: [(u64, &'static str); 4] = [(TIB, "TiB"), (GIB, "GiB"), (MIB, "MiB"), (KIB, "KiB")];
    for (div, unit) in steps {
        if bytes >= div {
            let whole = bytes.checked_div(div).unwrap_or(0);
            let rem = bytes.checked_rem(div).unwrap_or(0);
            // rem < div, so rem * 10 / div < 10; use u128 to avoid overflow.
            let tenth_wide = u128::from(rem)
                .saturating_mul(10)
                .checked_div(u128::from(div))
                .unwrap_or(0);
            let tenth = u8::try_from(tenth_wide).unwrap_or(9);
            return IecParts { whole, tenth, unit };
        }
    }
    IecParts {
        whole: bytes,
        tenth: 0,
        unit: "B",
    }
}

/// Parses a human size like `512K`, `1.5M`, `2GiB`, `100` (bytes). Returns `None` on overflow or garbage.
pub fn parse_size(text: &str) -> Option<u64> {
    let t = text.trim();
    let split = t
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(t.len());
    let (num, unit) = t.split_at(split);
    let mult = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "k" | "kb" | "kib" => KIB,
        "m" | "mb" | "mib" => MIB,
        "g" | "gb" | "gib" => GIB,
        "t" | "tb" | "tib" => TIB,
        _ => return None,
    };
    let (int_part, frac_part) = match num.split_once('.') {
        Some((i, f)) => (i, f),
        None => (num, ""),
    };
    if int_part.is_empty() || int_part.len() > 20 || frac_part.len() > 3 || frac_part.contains('.')
    {
        return None;
    }
    let whole: u64 = int_part.parse().ok()?;
    let mut total = whole.checked_mul(mult)?;
    if !frac_part.is_empty() {
        let frac: u64 = frac_part.parse().ok()?;
        let scale = 10u64.checked_pow(u32::try_from(frac_part.len()).ok()?)?;
        let add = mult.checked_mul(frac)?.checked_div(scale)?;
        total = total.checked_add(add)?;
    }
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iec_examples() {
        assert_eq!(
            iec_parts(0),
            IecParts {
                whole: 0,
                tenth: 0,
                unit: "B"
            }
        );
        assert_eq!(iec_parts(1_023).unit, "B");
        assert_eq!(
            iec_parts(1_536),
            IecParts {
                whole: 1,
                tenth: 5,
                unit: "KiB"
            }
        );
        assert_eq!(
            iec_parts(MIB * 3 / 2),
            IecParts {
                whole: 1,
                tenth: 5,
                unit: "MiB"
            }
        );
        assert_eq!(iec_parts(u64::MAX).unit, "TiB");
    }

    #[test]
    fn parse_sizes() {
        assert_eq!(parse_size("512K"), Some(512 * KIB));
        assert_eq!(parse_size("1.5M"), Some(MIB + MIB / 2));
        assert_eq!(parse_size("2 GiB"), Some(2 * GIB));
        assert_eq!(parse_size("100"), Some(100));
    }

    #[test]
    fn hostile_parse_size_overflow_and_garbage() {
        assert_eq!(parse_size("99999999999999999999T"), None);
        assert_eq!(parse_size("1.2.3M"), None);
        assert_eq!(parse_size("-1"), None);
        assert_eq!(parse_size("1X"), None);
        assert_eq!(parse_size(""), None);
    }
}
