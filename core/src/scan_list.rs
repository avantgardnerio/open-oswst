//! The menu's list of networks in range, as scans come in. Append only, so
//! the rows never move under the cursor: a network new to this list goes at
//! the bottom, one a scan no longer sees stays where it is (its signal shown
//! as unknown), and one seen again just gets its new signal.

use crate::devices::wifi::Seen;

pub struct Row {
    pub ssid: heapless::String<32>,
    pub rssi_dbm: Option<i8>, // None: the last scan didn't see it
    pub open: bool,
}

#[derive(Default)]
pub struct ScanList {
    rows: Vec<Row>,
    scanned: bool,
}

impl ScanList {
    /// Take one scan. Hidden networks (no name) are left out; a name seen
    /// more than once (several access points) counts its strongest
    pub fn add_scan(&mut self, found: &[Seen]) {
        self.scanned = true;
        for row in &mut self.rows {
            row.rssi_dbm = None;
        }
        for seen in found.iter().filter(|seen| !seen.ssid.is_empty()) {
            match self.rows.iter_mut().find(|row| row.ssid == seen.ssid) {
                Some(row) => {
                    row.rssi_dbm = Some(
                        row.rssi_dbm
                            .map_or(seen.rssi_dbm, |strongest| strongest.max(seen.rssi_dbm)),
                    );
                    row.open = seen.open;
                }
                None => self.rows.push(Row {
                    ssid: seen.ssid.clone(),
                    rssi_dbm: Some(seen.rssi_dbm),
                    open: seen.open,
                }),
            }
        }
    }

    /// Has any scan come in yet?
    pub fn scanned(&self) -> bool {
        self.scanned
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }
}

impl Row {
    /// "-61 Starlink", or " -- Starlink" when the last scan missed it
    pub fn label(&self) -> String {
        match self.rssi_dbm {
            Some(dbm) => format!("{:>3} {}", dbm, self.ssid),
            None => format!("{:>3} {}", "--", self.ssid),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(ssid: &str, rssi_dbm: i8) -> Seen {
        Seen {
            ssid: ssid.try_into().unwrap(),
            rssi_dbm,
            open: false,
        }
    }

    fn labels(list: &ScanList) -> Vec<String> {
        list.rows().iter().map(Row::label).collect()
    }

    #[test]
    fn rows_never_move() {
        let mut list = ScanList::default();
        assert!(!list.scanned());
        list.add_scan(&[seen("b", -70), seen("a", -50)]);
        assert_eq!(labels(&list), ["-70 b", "-50 a"]);
        // "a" gone, "c" new, "b" stronger: "c" goes last, "a" keeps its row
        list.add_scan(&[seen("c", -40), seen("b", -60)]);
        assert_eq!(labels(&list), ["-60 b", " -- a", "-40 c"]);
        assert!(list.scanned());
    }

    #[test]
    fn hidden_left_out_and_strongest_access_point_counts() {
        let mut list = ScanList::default();
        list.add_scan(&[seen("", -30), seen("mesh", -80), seen("mesh", -55)]);
        assert_eq!(labels(&list), ["-55 mesh"]);
    }
}
