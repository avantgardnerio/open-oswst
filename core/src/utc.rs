//! UTC calendar arithmetic, without a time crate: the GPS's date and time to
//! seconds since 1970, and back to the formats we print. Proleptic Gregorian
//! (Howard Hinnant's days_from_civil / civil_from_days).

/// Seconds since 1970-01-01T00:00:00Z for a UTC date (yyyy, mm, dd) and time
/// (hh, mm, ss)
pub fn unix_seconds(
    (year, month, day): (u16, u8, u8),
    (hour, minute, second): (u8, u8, u8),
) -> i64 {
    days_from_civil(year as i64, month as i64, day as i64) * 86_400
        + hour as i64 * 3600
        + minute as i64 * 60
        + second as i64
}

/// `2026-10-03T23:15:02Z`
pub fn iso(seconds: i64) -> String {
    let (year, month, day, hour, minute, second) = civil(seconds);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hour, minute, second
    )
}

/// `Sat, 03 Oct 2026 23:15:02 GMT`: the date format HTTP (and WebDAV) use
pub fn http_date(seconds: i64) -> String {
    const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let (year, month, day, hour, minute, second) = civil(seconds);
    let weekday = DAYS[seconds.div_euclid(86_400).rem_euclid(7) as usize]; // 1970-01-01: Thursday
    format!(
        "{}, {:02} {} {:04} {:02}:{:02}:{:02} GMT",
        weekday,
        day,
        MONTHS[month as usize - 1],
        year,
        hour,
        minute,
        second
    )
}

/// (year, month, day, hour, minute, second)
fn civil(seconds: i64) -> (i64, i64, i64, i64, i64, i64) {
    let days = seconds.div_euclid(86_400);
    let in_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    (
        year,
        month,
        day,
        in_day / 3600,
        in_day % 3600 / 60,
        in_day % 60,
    )
}

/// Days since 1970-01-01 for a date
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The date for days since 1970-01-01
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_instants() {
        assert_eq!(unix_seconds((1970, 1, 1), (0, 0, 0)), 0);
        // `date -u -d 2026-10-03T23:15:02Z +%s`
        assert_eq!(unix_seconds((2026, 10, 3), (23, 15, 2)), 1_791_069_302);
        // A leap day, and the day after
        assert_eq!(unix_seconds((2024, 2, 29), (12, 0, 0)), 1_709_208_000);
        assert_eq!(unix_seconds((2024, 3, 1), (0, 0, 0)), 1_709_251_200);
    }

    #[test]
    fn formats() {
        assert_eq!(iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso(1_791_069_302), "2026-10-03T23:15:02Z");
        assert_eq!(http_date(0), "Thu, 01 Jan 1970 00:00:00 GMT");
        assert_eq!(http_date(1_791_069_302), "Sat, 03 Oct 2026 23:15:02 GMT");
        assert_eq!(http_date(1_709_208_000), "Thu, 29 Feb 2024 12:00:00 GMT");
    }

    #[test]
    fn round_trips_every_day_for_a_century() {
        for days in 0..36_525 {
            let (year, month, day) = civil_from_days(days);
            assert_eq!(
                days_from_civil(year, month, day),
                days,
                "{year}-{month}-{day}"
            );
        }
    }
}
