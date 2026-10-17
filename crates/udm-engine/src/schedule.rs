//! Queue schedules: start/stop times of day, weekdays and the post-completion
//! action. Pure logic over local wall-clock times supplied by the caller.

use chrono::{Datelike, Duration as ChronoDuration, NaiveDateTime, NaiveTime};
use serde::{Deserialize, Serialize};
use udm_core::Message;

/// What to do when a scheduled queue has finished all its downloads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AfterDone {
    /// Nothing.
    #[default]
    None,
    /// Exit UDM.
    Exit,
    /// Shut down the computer.
    Shutdown,
    /// Put the computer to sleep.
    Sleep,
    /// Hibernate the computer.
    Hibernate,
}

impl AfterDone {
    /// Machine names accepted by `schedule.set`.
    pub const NAMES: &'static [&'static str] = &["none", "exit", "shutdown", "sleep", "hibernate"];

    /// Parses a machine name.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "none" => AfterDone::None,
            "exit" => AfterDone::Exit,
            "shutdown" => AfterDone::Shutdown,
            "sleep" => AfterDone::Sleep,
            "hibernate" => AfterDone::Hibernate,
            _ => return None,
        })
    }

    /// Machine name.
    pub const fn as_str(self) -> &'static str {
        match self {
            AfterDone::None => "none",
            AfterDone::Exit => "exit",
            AfterDone::Shutdown => "shutdown",
            AfterDone::Sleep => "sleep",
            AfterDone::Hibernate => "hibernate",
        }
    }
}

/// Schedule of one queue.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedule {
    /// Start time `HH:MM` (local).
    pub start_at: Option<String>,
    /// Stop time `HH:MM` (local).
    pub stop_at: Option<String>,
    /// Allowed weekdays, 0 = Monday … 6 = Sunday; empty = every day.
    #[serde(default)]
    pub days: Vec<u8>,
    /// Post-completion action.
    #[serde(default)]
    pub after_done: AfterDone,
}

/// Something the scheduler must do now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScheduleEvent {
    /// Start the queue.
    Start,
    /// Stop the queue.
    Stop,
}

/// Parses `HH:MM` (24 h).
pub fn parse_time_of_day(s: &str) -> Result<NaiveTime, Message> {
    let bad = || Message::new("error-schedule-bad-time").with_str("value", s);
    let (h, m) = s.trim().split_once(':').ok_or_else(bad)?;
    if h.is_empty() || h.len() > 2 || m.len() != 2 {
        return Err(bad());
    }
    let h: u32 = h.parse().map_err(|_| bad())?;
    let m: u32 = m.parse().map_err(|_| bad())?;
    NaiveTime::from_hms_opt(h, m, 0).ok_or_else(bad)
}

/// Parses weekday names/numbers (`mon,tue` or `0,1`) into 0..=6.
pub fn parse_days(items: &[String]) -> Result<Vec<u8>, Message> {
    let mut days = Vec::new();
    for item in items {
        let d = match item.trim().to_ascii_lowercase().as_str() {
            "0" | "mon" | "sen" => 0,
            "1" | "tue" | "sel" => 1,
            "2" | "wed" | "rab" => 2,
            "3" | "thu" | "kam" => 3,
            "4" | "fri" | "jum" => 4,
            "5" | "sat" | "sab" => 5,
            "6" | "sun" | "min" => 6,
            _ => {
                return Err(Message::new("error-schedule-bad-day").with_str("value", item.as_str()));
            }
        };
        if !days.contains(&d) {
            days.push(d);
        }
    }
    days.sort_unstable();
    Ok(days)
}

impl Schedule {
    /// Validates times and days.
    pub fn validate(&self) -> Result<(), Message> {
        for t in [&self.start_at, &self.stop_at].into_iter().flatten() {
            parse_time_of_day(t)?;
        }
        if self.days.iter().any(|d| *d > 6) {
            return Err(Message::new("error-schedule-bad-day").with_str("value", "?"));
        }
        Ok(())
    }

    fn day_allowed(&self, dt: NaiveDateTime) -> bool {
        let wd = u8::try_from(dt.weekday().num_days_from_monday()).unwrap_or(0);
        self.days.is_empty() || self.days.contains(&wd)
    }

    /// True if a boundary at `time` lies in `(prev, now]` on an allowed day.
    fn crossed(&self, time: Option<&String>, prev: NaiveDateTime, now: NaiveDateTime) -> bool {
        let Some(t) = time.and_then(|t| parse_time_of_day(t).ok()) else {
            return false;
        };
        if now <= prev {
            return false;
        }
        // Look at most two days back: long sleeps fire once, not once per missed day.
        let earliest = prev.max(
            now.checked_sub_signed(ChronoDuration::days(2))
                .unwrap_or(prev),
        );
        let mut day = earliest.date();
        while day <= now.date() {
            let candidate = day.and_time(t);
            if candidate > earliest && candidate <= now && self.day_allowed(candidate) {
                return true;
            }
            match day.succ_opt() {
                Some(d) => day = d,
                None => break,
            }
        }
        false
    }

    /// Events whose boundaries were crossed between two scheduler ticks.
    pub fn due_events(&self, prev: NaiveDateTime, now: NaiveDateTime) -> Vec<ScheduleEvent> {
        let mut out = Vec::new();
        if self.crossed(self.start_at.as_ref(), prev, now) {
            out.push(ScheduleEvent::Start);
        }
        if self.crossed(self.stop_at.as_ref(), prev, now) {
            out.push(ScheduleEvent::Stop);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(d: u32, h: u32, m: u32) -> NaiveDateTime {
        // 2026-10-12 is a Monday.
        NaiveDate::from_ymd_opt(2026, 10, d)
            .unwrap()
            .and_hms_opt(h, m, 0)
            .unwrap()
    }

    #[test]
    fn schedule_window() {
        let s = Schedule {
            start_at: Some("23:00".into()),
            stop_at: Some("05:00".into()),
            days: vec![],
            after_done: AfterDone::Sleep,
        };
        assert_eq!(
            s.due_events(at(12, 22, 59), at(12, 23, 0)),
            vec![ScheduleEvent::Start]
        );
        assert!(s.due_events(at(12, 23, 0), at(12, 23, 1)).is_empty());
        assert_eq!(
            s.due_events(at(13, 4, 59), at(13, 5, 0)),
            vec![ScheduleEvent::Stop]
        );
    }

    #[test]
    fn weekday_filter() {
        let s = Schedule {
            start_at: Some("08:00".into()),
            days: vec![1], // Tuesday
            ..Schedule::default()
        };
        assert!(s.due_events(at(12, 7, 0), at(12, 9, 0)).is_empty()); // Monday
        assert_eq!(
            s.due_events(at(13, 7, 0), at(13, 9, 0)),
            vec![ScheduleEvent::Start]
        );
    }

    #[test]
    fn hostile_schedule_bad_values() {
        for bad in ["24:00", "7", "07:5", "aa:bb", "-1:00", "07:60", ""] {
            assert!(parse_time_of_day(bad).is_err(), "{bad}");
        }
        assert!(parse_days(&["xyz".to_string()]).is_err());
        assert_eq!(
            parse_days(&["mon".into(), "0".into(), "sun".into()]).unwrap(),
            vec![0, 6]
        );
        // A clock jump of years still fires at most once and does not loop forever.
        let s = Schedule {
            start_at: Some("10:00".into()),
            ..Schedule::default()
        };
        let far = NaiveDate::from_ymd_opt(1970, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        assert_eq!(s.due_events(far, at(12, 11, 0)), vec![ScheduleEvent::Start]);
    }
}
