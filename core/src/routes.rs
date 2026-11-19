//! Where a path's input takes its sound from and where its output sends it, by unit.
//! The numbers are the ones .hlx files and the pedal keep; what they mean differs per unit.

use crate::snapshots::Family;

/// Input and output choices of a unit, as (value, name). `second_path` drops what only the
/// first path of a two-path unit can do (feeding the second path).
pub fn route_names(fam: Family, input: bool, second_path: bool) -> Vec<(i64, &'static str)> {
    if fam.paths == 1 {
        return if input {
            vec![(0, "None"), (1, "Main L/R"), (4, "Return L/R"), (6, "USB 5/6")]
        } else {
            vec![(0, "None"), (1, "Multi (Main L/R, Send, USB)")]
        };
    }
    let names: &[&str] = if input {
        &[
            "None",
            "Multi (Guitar, Aux, Variax)",
            "Guitar",
            "Aux",
            "Variax",
            "Variax Magnetics",
            "Mic",
            "Return 1",
            "Return 2",
            "Return 3",
            "Return 4",
            "Return 1/2",
            "Return 3/4",
            "S/PDIF",
            "USB 3/4",
            "USB 5/6",
            "USB 7/8",
        ]
    } else {
        &[
            "None",
            "Multi (1/4\", XLR, Digital, USB 1/2)",
            "Path 2A",
            "Path 2B",
            "Path 2A+B",
            "1/4\"",
            "XLR",
            "Send 1/2",
            "Send 3/4",
            "Digital",
            "USB 1/2",
            "USB 3/4",
            "USB 5/6",
        ]
    };
    // What the Helix LT's panel doesn't have: no Aux, Mic or S/PDIF input, two returns, two sends.
    let lt = fam.name == "Helix LT";
    let missing: &[i64] = match (lt, input) {
        (true, true) => &[3, 6, 9, 10, 12, 13],
        (true, false) => &[8],
        _ => &[],
    };
    names
        .iter()
        .enumerate()
        .map(|(i, n)| (i as i64, *n))
        .filter(|(v, _)| !missing.contains(v))
        .filter(|(v, _)| input || !((2..=4).contains(v) && second_path))
        .collect()
}

/// The name of a route value on a unit (any path), if the unit has it.
pub fn route_name(fam: Family, input: bool, value: i64) -> Option<&'static str> {
    [false, true].into_iter().find_map(|second| route_names(fam, input, second).into_iter().find(|(v, _)| *v == value).map(|(_, n)| n))
}

/// The same route on another unit: (value there, whether it is an exact match). A route the
/// target doesn't have becomes its nearest equivalent (the main input or output, a return pair).
pub fn map_route(from: Family, to: Family, input: bool, value: i64) -> (i64, bool) {
    if value == 0 {
        return (0, true);
    }
    let Some(name) = route_name(from, input, value) else { return (1, false) };
    // The same name on the target.
    if let Some((v, _)) = [false, true].into_iter().flat_map(|second| route_names(to, input, second)).find(|(_, n)| *n == name) {
        return (v, true);
    }
    // The nearest equivalent.
    let want = if input {
        if name.starts_with("Return") {
            "Return"
        } else if name.starts_with("USB") {
            "USB"
        } else {
            "Main"
        }
    } else {
        "Multi"
    };
    let near = [false, true].into_iter().flat_map(|second| route_names(to, input, second)).find(|(_, n)| match want {
        "Return" => *n == "Return L/R" || *n == "Return 1/2",
        "USB" => n.starts_with("USB"),
        "Main" => *n == "Main L/R" || n.starts_with("Multi"),
        _ => n.starts_with("Multi"),
    });
    // A Stomp's Main L/R and a Helix's Multi stand for each other, as do the return pairs.
    let exact = match want {
        "Return" => matches!(name, "Return L/R" | "Return 1/2"),
        "USB" => false,
        "Main" => name.starts_with("Multi") || name == "Main L/R",
        _ => name.starts_with("Multi"),
    };
    match near {
        Some((v, _)) => (v, exact),
        None => (1, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshots::family_of_device;

    #[test]
    fn units_list_only_the_jacks_they_have() {
        let lt = family_of_device(2162692);
        assert!(!route_names(lt, true, false).iter().any(|(_, n)| *n == "Mic" || *n == "Aux"));
        assert!(!route_names(lt, false, false).iter().any(|(_, n)| *n == "Send 3/4"));
        let floor = family_of_device(2162689);
        assert!(route_names(floor, true, false).iter().any(|(_, n)| *n == "Mic"));
        // Only Path 1 can feed Path 2.
        assert!(route_names(floor, false, false).iter().any(|(_, n)| *n == "Path 2A"));
        assert!(!route_names(floor, false, true).iter().any(|(_, n)| *n == "Path 2A"));
        // One-path units (Effects included) get the short list.
        assert_eq!(route_names(family_of_device(2162693), true, false).len(), 4);
    }

    #[test]
    fn routes_follow_their_meaning_between_units() {
        let stomp = family_of_device(2162694);
        let floor = family_of_device(2162689);
        let lt = family_of_device(2162692);
        // Stomp "Return L/R" (4) is the Helix's "Return 1/2" (11), not its Variax (4).
        assert_eq!(map_route(stomp, floor, true, 4), (11, true));
        assert_eq!(map_route(floor, stomp, true, 11), (4, true));
        // USB 5/6 matches by name.
        assert_eq!(map_route(stomp, floor, true, 6), (15, true));
        // The Helix's Mic has no equivalent on a Stomp: the main input, flagged.
        assert_eq!(map_route(floor, stomp, true, 6), (1, false));
        // Floor's Mic isn't on an LT.
        assert_eq!(map_route(floor, lt, true, 6), (1, false));
        // Outputs: Multi stays Multi; a specific jack falls back to it, flagged.
        assert_eq!(map_route(floor, stomp, false, 1), (1, true));
        assert_eq!(map_route(floor, stomp, false, 6), (1, false));
        assert_eq!(map_route(stomp, floor, true, 0), (0, true));
    }
}
