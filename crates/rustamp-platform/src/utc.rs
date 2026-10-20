// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The UTC calendar: a unix second as a date and a time of day.
//!
//! The player shows times in **UTC**, so a time does not depend on the time
//! zone of the machine that shows it, and two machines show the same text for
//! the same moment.
//!
//! The conversion is the era-based one: the 400-year era, the day of the era
//! and the day of the year give the year, the month and the day. There is no
//! lookup table and no dependency, so every date works and the same answer
//! comes out on every platform.

/// A moment in time, as the UTC calendar sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UtcCalendar {
    /// The year of the proleptic Gregorian calendar.
    pub year: i64,
    /// The month, 1 to 12.
    pub month: u32,
    /// The day of the month, 1 to 31.
    pub day: u32,
    /// The hour, 0 to 23.
    pub hour: u32,
    /// The minute, 0 to 59.
    pub minute: u32,
    /// The second, 0 to 59.
    pub second: u32,
}

/// The UTC calendar of the unix second `seconds`.
///
/// The seconds before the epoch are the ones before 1970-01-01: the calendar
/// walks back over them like it walks forward.
pub fn utc_calendar(seconds: i64) -> UtcCalendar {
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_date(days);
    UtcCalendar {
        year,
        month,
        day,
        hour: (rest / 3_600) as u32,
        minute: (rest % 3_600 / 60) as u32,
        second: (rest % 60) as u32,
    }
}

/// The calendar date of the day `days` after 1970-01-01.
///
/// The era-based conversion: the shifted day count is split into the
/// 400-year era, the day of the era and the day of the year, and the month
/// follows from the day of the year.
fn civil_date(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_of_year = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_of_year + 2) / 5 + 1) as u32;
    let month = if month_of_year < 10 {
        month_of_year + 3
    } else {
        month_of_year - 9
    } as u32;
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_epoch_is_the_first_of_january_1970() {
        assert_eq!(
            utc_calendar(0),
            UtcCalendar {
                year: 1970,
                month: 1,
                day: 1,
                hour: 0,
                minute: 0,
                second: 0,
            }
        );
    }

    #[test]
    fn a_known_moment_is_the_known_calendar() {
        // 2023-11-14 22:13:20 UTC.
        let at = utc_calendar(1_700_000_000);
        assert_eq!((at.year, at.month, at.day), (2023, 11, 14));
        assert_eq!((at.hour, at.minute, at.second), (22, 13, 20));
    }

    #[test]
    fn a_leap_day_is_a_day_of_its_month() {
        // 2024-02-29 12:00:00 UTC.
        let at = utc_calendar(1_709_208_000);
        assert_eq!((at.year, at.month, at.day), (2024, 2, 29));
        assert_eq!((at.hour, at.minute, at.second), (12, 0, 0));
    }

    #[test]
    fn the_seconds_before_the_epoch_walk_back_over_the_calendar() {
        let at = utc_calendar(-1);
        assert_eq!((at.year, at.month, at.day), (1969, 12, 31));
        assert_eq!((at.hour, at.minute, at.second), (23, 59, 59));
    }

    #[test]
    fn every_day_of_a_year_is_a_day_of_its_month() {
        // 2026-01-01 is the day 20_454 after the epoch.
        let start = 20_454 * 86_400;
        let first = utc_calendar(start);
        assert_eq!((first.year, first.month, first.day), (2026, 1, 1));

        // The year 2026 has 365 days: every one of them is a date the
        // calendar must name, and the year must not run into 2027.
        for day in 0..365 {
            let at = utc_calendar(start + day * 86_400);
            assert_eq!(at.year, 2026, "day {day}");
            assert!((1..=12).contains(&at.month), "day {day}");
            assert!((1..=31).contains(&at.day), "day {day}");
        }
        let after = utc_calendar(start + 365 * 86_400);
        assert_eq!((after.year, after.month, after.day), (2027, 1, 1));
    }
}
