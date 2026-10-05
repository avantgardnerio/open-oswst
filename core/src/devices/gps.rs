//! GPS: the latest time and position, and the NMEA parsing that produces it.
//!
//! Only two sentences matter: RMC (UTC time, date, position, valid flag) and
//! GGA (satellites in use). A GPS usually knows the time well before it has a
//! position, so the two are reported separately.

/// Something that knows where and when we are.
pub trait Gps {
    /// Latest fix, or None if the GPS has gone quiet (unplugged, or never started).
    fn latest(&self) -> Option<Fix>;
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Fix {
    pub time: Option<(u8, u8, u8)>,   // UTC hh, mm, ss
    pub date: Option<(u16, u8, u8)>,  // yyyy, mm, dd
    pub position: Option<(f64, f64)>, // lat, lon in degrees; None without a fix
    pub satellites: u8,               // used in the fix (GGA)
    /// In view, per system (GSV): GPS, BeiDou, GLONASS. On our L76K these
    /// are the satellites it hears: the count climbs as it acquires, while
    /// `satellites` stays 0 until the fix
    pub in_view: [u8; 3],
}

impl Fix {
    /// Satellites in view, all systems
    pub fn in_view(&self) -> u8 {
        self.in_view.iter().sum()
    }
}

/// `2026-09-30T20:49:00Z 40.54235,-105.08326 sats=7`, with `no-fix` for the
/// position and `no-time` / `no-date` for anything the GPS doesn't know yet.
impl std::fmt::Display for Fix {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self.date {
            Some((y, mo, d)) => write!(f, "{:04}-{:02}-{:02}T", y, mo, d)?,
            None => write!(f, "no-date ")?,
        }
        match self.time {
            Some((h, m, s)) => write!(f, "{:02}:{:02}:{:02}Z", h, m, s)?,
            None => write!(f, "no-time")?,
        }
        match self.position {
            Some((lat, lon)) => write!(f, " {:.5},{:.5}", lat, lon)?,
            None => write!(f, " no-fix")?,
        }
        write!(f, " sats={}/{}", self.satellites, self.in_view())
    }
}

/// A text command for the GPS: `$<body>*<checksum>\r\n`, the checksum the
/// XOR of the body's bytes. e.g. `PCAS06,0`: report the firmware version
pub fn command(body: &str) -> Vec<u8> {
    let sum = body.bytes().fold(0u8, |sum, byte| sum ^ byte);
    format!("${}*{:02X}\r\n", body, sum).into_bytes()
}

/// Fold one NMEA sentence into `fix`. Returns whether it was one we use.
pub fn apply_nmea(fix: &mut Fix, sentence: &str) -> bool {
    let Some(body) = checked_body(sentence) else {
        return false;
    };
    let fields: Vec<&str> = body.split(',').collect();
    match fields[0].get(2..) {
        // $GNRMC,hhmmss.sss,A|V,lat,N|S,lon,E|W,speed,course,ddmmyy,...
        Some("RMC") if fields.len() > 9 => {
            fix.time = parse_time(fields[1]);
            fix.date = parse_date(fields[9]);
            fix.position = if fields[2] == "A" {
                parse_coord(fields[3], fields[4]).zip(parse_coord(fields[5], fields[6]))
            } else {
                None
            };
            true
        }
        // $GNGGA,time,lat,N,lon,E,quality,satellites,...
        Some("GGA") if fields.len() > 7 => {
            fix.satellites = fields[7].parse().unwrap_or(0);
            true
        }
        // $GPGSV,messages,number,in view,... (one set per system)
        Some("GSV") if fields.len() > 3 => {
            let system = match fields[0].get(..2) {
                Some("GP") => 0,
                Some("BD" | "GB") => 1,
                Some("GL") => 2,
                _ => return false,
            };
            fix.in_view[system] = fields[3].parse().unwrap_or(0);
            true
        }
        _ => false,
    }
}

/// The part between `$` and `*`, if the checksum after `*` matches.
fn checked_body(sentence: &str) -> Option<&str> {
    let (body, checksum) = sentence.strip_prefix('$')?.split_once('*')?;
    let expected = u8::from_str_radix(checksum, 16).ok()?;
    let actual = body.bytes().fold(0u8, |sum, b| sum ^ b);
    (actual == expected).then_some(body)
}

/// "204900.000" -> (20, 49, 0)
fn parse_time(field: &str) -> Option<(u8, u8, u8)> {
    let digits = field.get(..6)?;
    Some((
        digits[0..2].parse().ok()?,
        digits[2..4].parse().ok()?,
        digits[4..6].parse().ok()?,
    ))
}

/// "300926" -> (2026, 9, 30)
fn parse_date(field: &str) -> Option<(u16, u8, u8)> {
    if field.len() != 6 {
        return None;
    }
    Some((
        2000 + field[4..6].parse::<u16>().ok()?,
        field[2..4].parse().ok()?,
        field[0..2].parse().ok()?,
    ))
}

/// NMEA (d)ddmm.mmmm plus hemisphere -> signed degrees. The degrees are
/// everything before the last two digits ahead of the '.'.
fn parse_coord(value: &str, hemisphere: &str) -> Option<f64> {
    let dot = value.find('.')?;
    let degrees: f64 = value.get(..dot.checked_sub(2)?)?.parse().ok()?;
    let minutes: f64 = value.get(dot - 2..)?.parse().ok()?;
    let magnitude = degrees + minutes / 60.0;
    match hemisphere {
        "N" | "E" => Some(magnitude),
        "S" | "W" => Some(-magnitude),
        _ => None,
    }
}

/// Great-circle distance between two positions (lat, lon in degrees), in
/// metres: haversine on a sphere of the Earth's mean radius. Within 0.5% of
/// the true (ellipsoid) distance, plenty for "how far away were they"
pub fn distance_m(a: (f64, f64), b: (f64, f64)) -> f64 {
    const EARTH_RADIUS_M: f64 = 6_371_008.8;
    let (lat1, lon1) = (a.0.to_radians(), a.1.to_radians());
    let (lat2, lon2) = (b.0.to_radians(), b.1.to_radians());
    let h = ((lat2 - lat1) / 2.0).sin().powi(2)
        + lat1.cos() * lat2.cos() * ((lon2 - lon1) / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_M * h.sqrt().asin()
}

/// The direction from `from` to `to` (lat, lon in degrees), in degrees
/// clockwise from true north, 0 to 360: the great-circle initial bearing,
/// the way you'd set off
pub fn bearing_deg(from: (f64, f64), to: (f64, f64)) -> f64 {
    let (lat1, lon1) = (from.0.to_radians(), from.1.to_radians());
    let (lat2, lon2) = (to.0.to_radians(), to.1.to_radians());
    let east = (lon2 - lon1).sin() * lat2.cos();
    let north = lat1.cos() * lat2.sin() - lat1.sin() * lat2.cos() * (lon2 - lon1).cos();
    east.atan2(north).to_degrees().rem_euclid(360.0)
}

/// A bearing as the nearest of the compass's 16 points: N, NNE, NE, ENE...
pub fn compass_point(bearing_deg: f64) -> &'static str {
    const POINTS: [&str; 16] = [
        "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW",
        "NW", "NNW",
    ];
    let point = (bearing_deg.rem_euclid(360.0) / 22.5).round() as usize % POINTS.len();
    POINTS[point]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Append the right checksum, as a GPS would
    fn sentence(body: &str) -> String {
        let sum = body.bytes().fold(0u8, |sum, b| sum ^ b);
        format!("${}*{:02X}", body, sum)
    }

    #[test]
    fn rmc_with_a_fix() {
        let mut fix = Fix::default();
        let rmc = sentence("GNRMC,205546.000,A,4032.6418,N,10505.5182,W,0.00,0.00,300926,,,A,V");
        assert!(apply_nmea(&mut fix, &rmc));
        assert_eq!(fix.time, Some((20, 55, 46)));
        assert_eq!(fix.date, Some((2026, 9, 30)));
        let (lat, lon) = fix.position.unwrap();
        assert!((lat - 40.54403).abs() < 1e-5, "lat {}", lat);
        assert!((lon - -105.09197).abs() < 1e-5, "lon {}", lon);
    }

    #[test]
    fn rmc_without_a_fix_has_no_position() {
        let mut fix = Fix::default();
        assert!(apply_nmea(&mut fix, &sentence("GNRMC,,V,,,,,,,,,,N,V")));
        assert_eq!(fix, Fix::default());
    }

    #[test]
    fn gga_gives_satellites() {
        let mut fix = Fix::default();
        let gga = sentence("GNGGA,205546.000,4032.6418,N,10505.5182,W,1,06,1.2,1520.0,M,0.0,M,,");
        assert!(apply_nmea(&mut fix, &gga));
        assert_eq!(fix.satellites, 6);
    }

    #[test]
    fn bad_checksum_is_ignored() {
        let mut fix = Fix::default();
        assert!(!apply_nmea(&mut fix, "$GNGGA,,,,,,0,07,,,,,,,*00"));
        assert_eq!(fix.satellites, 0);
    }

    #[test]
    fn other_sentences_are_ignored() {
        let mut fix = Fix::default();
        assert!(!apply_nmea(&mut fix, &sentence("GPTXT,01,01,02,MA=CASIC")));
    }

    #[test]
    fn display_is_the_log_format() {
        let fix = Fix {
            time: Some((20, 55, 46)),
            date: Some((2026, 9, 30)),
            position: Some((40.54403, -105.09197)),
            satellites: 6,
            in_view: [9, 4, 0],
        };
        assert_eq!(
            fix.to_string(),
            "2026-09-30T20:55:46Z 40.54403,-105.09197 sats=6/13"
        );
        assert_eq!(
            Fix::default().to_string(),
            "no-date no-time no-fix sats=0/0"
        );
    }

    #[test]
    fn commands_carry_their_checksum() {
        // docs/Quectel_L76K_GNSS_protocol_specification_V1.1.pdf, PCAS10
        assert_eq!(command("PCAS10,0"), b"$PCAS10,0*1C\r\n");
    }

    #[test]
    fn gsv_gives_satellites_in_view_per_system() {
        let mut fix = Fix::default();
        let gps = sentence("GPGSV,3,1,11,02,48,107,,05,30,296,,13,40,050,,15,62,215,,0");
        assert!(apply_nmea(&mut fix, &gps));
        let beidou = sentence("BDGSV,1,1,03,06,30,180,,09,41,220,,16,20,100,,0");
        assert!(apply_nmea(&mut fix, &beidou));
        assert_eq!(fix.in_view, [11, 3, 0]);
        assert_eq!(fix.in_view(), 14);
    }

    #[test]
    fn distances_match_known_ones() {
        // One degree of latitude: ~111.2 km
        assert!((distance_m((40.0, -105.0), (41.0, -105.0)) - 111_195.0).abs() < 50.0);
        // Walk 6's far end to home, along the street: ~1.35 km
        let d = distance_m((40.545_389, -105.107_722), (40.543_9, -105.091_85));
        assert!((d - 1_350.0).abs() < 30.0, "{}", d);
        assert_eq!(distance_m((40.5, -105.1), (40.5, -105.1)), 0.0);
    }

    #[test]
    fn bearings_point_the_right_way() {
        let here = (0.0, 0.0);
        let near = |bearing: f64, expected: f64| (bearing - expected).abs() < 0.01;
        assert!(near(bearing_deg(here, (0.01, 0.0)), 0.0));
        assert!(near(bearing_deg(here, (0.0, 0.01)), 90.0));
        assert!(near(bearing_deg(here, (-0.01, 0.0)), 180.0));
        assert!(near(bearing_deg(here, (0.0, -0.01)), 270.0));
        assert!(near(bearing_deg(here, (0.01, 0.01)), 45.0));
        // Across the date line: a short hop east, not most of the way round
        assert!(near(bearing_deg((0.0, 179.99), (0.0, -179.99)), 90.0));
    }

    #[test]
    fn compass_points_are_the_nearest_of_sixteen() {
        assert_eq!(compass_point(0.0), "N");
        assert_eq!(compass_point(11.0), "N");
        assert_eq!(compass_point(12.0), "NNE");
        assert_eq!(compass_point(45.0), "NE");
        assert_eq!(compass_point(90.0), "E");
        assert_eq!(compass_point(202.5), "SSW");
        assert_eq!(compass_point(337.5), "NNW");
        assert_eq!(compass_point(349.0), "N"); // rounds round to north
        assert_eq!(compass_point(360.0), "N");
    }
}
