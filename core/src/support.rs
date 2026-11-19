//! What has been checked on which unit. Shown the first time a unit is connected and put in problem reports,
//! so a person knows what to trust and what to check, and a report says what was in play.

use crate::snapshots;

/// How far an area has been tried.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    /// Tried on a real pedal of this unit.
    Checked,
    /// Built from Line 6's documented layout or another unit's behaviour; not tried on this unit.
    Documented,
}

/// An area of the app and how far it has been tried on a unit.
pub struct Area {
    pub name: &'static str,
    pub level: Level,
}

const AREAS: [&str; 8] = [
    "Reading and writing presets",
    "Live editing of blocks and knobs",
    "Snapshots",
    "Controllers and footswitches",
    "Command Center",
    "Backups and restore",
    "Guitar input (In-Z and pad)",
    "Setlists",
];

/// Whether a unit has been tried on a real pedal. Only the HX Stomp has.
pub fn checked_unit(device: i64) -> bool {
    device == 2162694
}

/// The areas of the app for a unit and how far each has been tried.
pub fn areas(device: i64) -> Vec<Area> {
    let level = if checked_unit(device) { Level::Checked } else { Level::Documented };
    AREAS.iter().map(|name| Area { name, level }).collect()
}

/// A few plain lines for a report or a notice.
pub fn summary(device: i64) -> String {
    let name = crate::units::unit_name(device);
    let fam = snapshots::family_of_device(device);
    let mut out = format!("{name} (device {device})\n");
    if checked_unit(device) {
        out.push_str("Tried on a real pedal: ");
        out.push_str(&AREAS.join(", "));
        out.push_str(".\n");
    } else {
        out.push_str("Not tried on a real pedal. Everything for this unit is built from the documented layouts ");
        out.push_str("(paths, slots, jacks, Command Center slots, snapshots) and from the HX Stomp's behaviour.\n");
    }
    if !crate::cmdslots::layout_verified(fam) {
        out.push_str("Command Center layout: from the documentation, not checked.\n");
    }
    out
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian.
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// "2026-11-18 0142" for a time (UTC), for a report's folder name.
pub fn stamp(now: std::time::SystemTime) -> String {
    let secs = now.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (y, m, d) = civil(secs.div_euclid(86_400));
    let rem = secs.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02} {:02}{:02}", rem / 3600, rem % 3600 / 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_stomp_counts_as_checked() {
        assert!(areas(2162694).iter().all(|a| a.level == Level::Checked));
        assert!(areas(2162699).iter().all(|a| a.level == Level::Documented));
        assert!(summary(2162694).contains("Tried on a real pedal"));
        let other = summary(2162689);
        assert!(other.contains("Not tried"), "{other}");
        assert!(other.contains("Command Center layout"), "{other}");
    }

    #[test]
    fn dates_are_right() {
        use std::time::{Duration, UNIX_EPOCH};
        assert_eq!(stamp(UNIX_EPOCH), "1970-01-01 0000");
        assert_eq!(stamp(UNIX_EPOCH + Duration::from_secs(1_794_955_402)), "2026-11-17 2243");
        assert_eq!(stamp(UNIX_EPOCH + Duration::from_secs(951_782_400 + 3 * 3600 + 7 * 60)), "2000-02-29 0307");
    }
}
