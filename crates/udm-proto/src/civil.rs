//! Proleptic Gregorian calendar math (UTC), fully checked.
//! Algorithms after H. Hinnant's public-domain "chrono-compatible low-level date algorithms".

/// Days since 1970-01-01 for a civil date; `None` on overflow or invalid date.
pub(crate) fn days_from_civil(year: i64, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
        return None;
    }
    let m = i64::from(month);
    let d = i64::from(day);
    let y = if m <= 2 { year.checked_sub(1)? } else { year };
    let era = y
        .checked_sub(if y >= 0 { 0 } else { 399 })?
        .checked_div(400)?;
    let yoe = y.checked_sub(era.checked_mul(400)?)?; // [0, 399]
    let mp = if m > 2 {
        m.checked_sub(3)?
    } else {
        m.checked_add(9)?
    }; // [0, 11]
    let doy = mp
        .checked_mul(153)?
        .checked_add(2)?
        .checked_div(5)?
        .checked_add(d)?
        .checked_sub(1)?; // [0, 365]
    let doe = yoe
        .checked_mul(365)?
        .checked_add(yoe.checked_div(4)?)?
        .checked_sub(yoe.checked_div(100)?)?
        .checked_add(doy)?; // [0, 146096]
    era.checked_mul(146_097)?
        .checked_add(doe)?
        .checked_sub(719_468)
}

/// Civil date `(year, month, day)` for days since 1970-01-01.
pub(crate) fn civil_from_days(days: i64) -> Option<(i64, u32, u32)> {
    let z = days.checked_add(719_468)?;
    let era = z
        .checked_sub(if z >= 0 { 0 } else { 146_096 })?
        .checked_div(146_097)?;
    let doe = z.checked_sub(era.checked_mul(146_097)?)?; // [0, 146096]
    let yoe = doe
        .checked_sub(doe.checked_div(1_460)?)?
        .checked_add(doe.checked_div(36_524)?)?
        .checked_sub(doe.checked_div(146_096)?)?
        .checked_div(365)?; // [0, 399]
    let y = yoe.checked_add(era.checked_mul(400)?)?;
    let doy = doe.checked_sub(
        yoe.checked_mul(365)?
            .checked_add(yoe.checked_div(4)?)?
            .checked_sub(yoe.checked_div(100)?)?,
    )?; // [0, 365]
    let mp = doy.checked_mul(5)?.checked_add(2)?.checked_div(153)?; // [0, 11]
    let d = doy
        .checked_sub(mp.checked_mul(153)?.checked_add(2)?.checked_div(5)?)?
        .checked_add(1)?;
    let m = if mp < 10 {
        mp.checked_add(3)?
    } else {
        mp.checked_sub(9)?
    };
    let y = if m <= 2 { y.checked_add(1)? } else { y };
    Some((y, u32::try_from(m).ok()?, u32::try_from(d).ok()?))
}

fn is_leap(year: i64) -> bool {
    (year.rem_euclid(4) == 0 && year.rem_euclid(100) != 0) || year.rem_euclid(400) == 0
}

pub(crate) fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Unix seconds for a UTC civil date-time; validates every field (second ≤ 60 for leap seconds).
pub(crate) fn unix_from_civil(
    year: i64,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
) -> Option<i64> {
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let days = days_from_civil(year, month, day)?;
    let secs = i64::from(hour)
        .checked_mul(3_600)?
        .checked_add(i64::from(minute).checked_mul(60)?)?
        .checked_add(i64::from(second))?;
    days.checked_mul(86_400)?.checked_add(secs)
}

/// Month number (1..=12) for an English three-letter abbreviation, case-insensitive.
pub(crate) fn month_from_abbr(s: &str) -> Option<u32> {
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    MONTHS
        .iter()
        .position(|m| m.eq_ignore_ascii_case(s))
        .and_then(|i| u32::try_from(i).ok())
        .and_then(|i| i.checked_add(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_and_round_trip() {
        assert_eq!(days_from_civil(1970, 1, 1), Some(0));
        assert_eq!(days_from_civil(2000, 3, 1), Some(11_017));
        assert_eq!(days_from_civil(1969, 12, 31), Some(-1));
        for d in [-800_000i64, -1, 0, 1, 59, 60, 10_957, 2_932_896] {
            let (y, m, dd) = civil_from_days(d).unwrap();
            assert_eq!(days_from_civil(y, m, dd), Some(d));
        }
        assert_eq!(days_from_civil(2023, 2, 29), None);
        assert_eq!(days_from_civil(2024, 2, 29), Some(19_782));
        assert_eq!(days_from_civil(i64::MAX, 1, 1), None);
    }
}
