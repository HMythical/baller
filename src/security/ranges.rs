//! Deciding whether an installed version falls inside an advisory's range.
//!
//! OSV describes affected versions two ways, and an entry may use either or
//! both: an explicit `versions` list, and `ranges` built from `introduced` /
//! `fixed` / `last_affected` events. This module normalises both into closed or
//! half-open intervals and answers one question — does this version land inside
//! any of them?
//!
//! Two rules shape everything here:
//!
//! * **An explicit `versions` list is believed over a computed range.** It is
//!   the publisher stating exact releases, so when one is present and the
//!   version is in it, that is a hit with no interpretation applied.
//! * **An unparseable version is never a silent miss.** Versions fall back to
//!   exact string comparison rather than being dropped, because "we could not
//!   read it" must not become "it is not affected".

use semver::Version;

use crate::core::dep_solver::parse_version_flexible;
use crate::security::identity::{ECOSYSTEM_DEBIAN, ECOSYSTEM_FEDORA};
use crate::security::osv::{Affected, Event, Range};

/// OSV ecosystems whose versions are distribution revisions rather than
/// semver. A Debian `1.2.3-7` *parses* as semver — with `7` read as a
/// pre-release — so these ecosystems must never reach the precise parser:
/// `1.2.3-7` is the seventh packaging of 1.2.3, not a release candidate of it.
const DISTRO_ECOSYSTEMS: &[&str] = &[
    ECOSYSTEM_DEBIAN,
    ECOSYSTEM_FEDORA,
    "Ubuntu",
    "Alpine",
    "Red Hat",
    "AlmaLinux",
    "Rocky Linux",
    "openSUSE",
    "SUSE",
];

/// Where a range stops: `fixed` excludes its own version, `last_affected`
/// includes it.
#[derive(Debug, Clone, PartialEq)]
enum Bound {
    /// `>= introduced`, with no upper limit
    Open,
    /// `>= introduced AND < fixed`
    Before(Version),
    /// `>= introduced AND <= last_affected`
    Through(Version),
}

#[derive(Debug, Clone, PartialEq)]
struct Interval {
    from: Version,
    to: Bound,
}

impl Interval {
    fn contains(&self, version: &Version) -> bool {
        if version < &self.from {
            return false;
        }
        match &self.to {
            Bound::Open => true,
            Bound::Before(end) => version < end,
            Bound::Through(end) => version <= end,
        }
    }
}

/// Whether `version` of `(ecosystem, name)` is affected by any of these entries.
pub fn affects(affected: &[Affected], ecosystem: &str, name: &str, version: &str) -> bool {
    affected
        .iter()
        .any(|entry| entry_affects(entry, ecosystem, name, version))
}

/// Whether one `affected` entry covers this version, and is about this package.
pub fn entry_affects(entry: &Affected, ecosystem: &str, name: &str, version: &str) -> bool {
    entry_is_about(entry, ecosystem, name)
        && covers_version(entry, version, !is_distro_ecosystem(ecosystem))
}

/// Whether an entry's versions and ranges include this version.
///
/// Deliberately says nothing about *which* package the entry describes. It is
/// used on its own when the package is already known to be the subject — a
/// self-declared advisory alias, where the author has asserted that the record
/// applies and the only open question is which versions it applies to.
///
/// With no query ecosystem to go on, the entry's own `package.ecosystem`
/// decides whether the installed version is read as semver or as a
/// distribution revision.
pub fn entry_covers_version(entry: &Affected, version: &str) -> bool {
    let precise = entry
        .package
        .as_ref()
        .is_none_or(|package| !is_distro_ecosystem(&package.ecosystem));
    covers_version(entry, version, precise)
}

/// The body of [`entry_covers_version`], with the parser choice made by the
/// caller. `precise` selects [`parse_advisory_version`] for the installed
/// version; distro ecosystems pass `false` and keep the lossy parser.
fn covers_version(entry: &Affected, version: &str, precise: bool) -> bool {
    // The publisher naming exact releases outranks anything computed.
    if version_listed(&entry.versions, version) {
        return true;
    }

    let parsed = if precise {
        parse_advisory_version(version)
    } else {
        parse_version_flexible(version)
    };
    let parsed = match parsed {
        Some(parsed) => parsed,
        // Nothing comparable to compare against: the explicit list above was
        // this version's only chance of a match.
        None => return false,
    };

    entry
        .ranges
        .iter()
        .any(|range| range_contains(range, &parsed))
}

/// Whether an entry carries no version information at all.
///
/// Such an entry states that a package is affected without narrowing it to any
/// release, so it cannot be excluded by version.
pub fn entry_has_no_version_bound(entry: &Affected) -> bool {
    entry.ranges.is_empty() && entry.versions.is_empty()
}

/// Whether an entry describes the package being checked.
///
/// An entry with no `package` block is taken to be about the record's own
/// subject — that is how single-package advisories are commonly written, and
/// discarding them would drop real hits. Ecosystems compare on the part before
/// `:`, because OSV suffixes distro releases (`Debian:11`, `Alpine:v3.19`) onto
/// a name Referee only knows unsuffixed.
pub fn entry_is_about(entry: &Affected, ecosystem: &str, name: &str) -> bool {
    let package = match entry.package.as_ref() {
        Some(package) => package,
        None => return true,
    };

    if !package.name.is_empty() && !package.name.eq_ignore_ascii_case(name) {
        return false;
    }

    if package.ecosystem.is_empty() {
        return true;
    }

    ecosystem_root(&package.ecosystem).eq_ignore_ascii_case(ecosystem_root(ecosystem))
}

/// Whether an ecosystem versions packages as distribution revisions.
fn is_distro_ecosystem(ecosystem: &str) -> bool {
    let root = ecosystem_root(ecosystem);
    DISTRO_ECOSYSTEMS
        .iter()
        .any(|distro| distro.eq_ignore_ascii_case(root))
}

/// Parse an installed version for advisory matching, keeping its pre-release.
///
/// [`parse_version_flexible`] strips pre-release and build metadata because a
/// distro version is not comparable as semver at all. For advisory matching
/// that trade is backwards: `1.2.3-rc1` and `1.2.3` are different releases,
/// and collapsing them puts an rc *outside* a range that is `fixed` at 1.2.3 —
/// reporting `clean` for a version inside the vulnerable window. A strict
/// semver parse that carries a pre-release is therefore returned as-is; every
/// other input, including bare `1.2.3+build` and every distro shape, takes the
/// lossy path it always took.
pub(crate) fn parse_advisory_version(raw: &str) -> Option<Version> {
    let trimmed = raw.trim();
    match Version::parse(trimmed) {
        Ok(version) if !version.pre.is_empty() => Some(version),
        _ => parse_version_flexible(trimmed),
    }
}

/// `Debian:11` and `Debian` are the same ecosystem for Referee's purposes.
fn ecosystem_root(ecosystem: &str) -> &str {
    ecosystem.split(':').next().unwrap_or(ecosystem).trim()
}

/// Whether an explicit `versions` list names this version.
///
/// Compared as written first, then as both sides normalise, so that a record
/// listing `1.21-76` still matches an installed `1.21`.
///
/// This deliberately stays on the lossy [`parse_version_flexible`] rather than
/// [`parse_advisory_version`]. A precise comparison could only *remove* hits —
/// a record listing `1.2.3` would stop matching an installed `1.2.3-rc1` — and
/// an explicit list is the publisher's own statement, so losing a hit from it
/// would loosen a block. Two versions equal under the precise parser are
/// always equal under the lossy one, so the lossy comparison alone already
/// matches "either way".
fn version_listed(versions: &[String], version: &str) -> bool {
    if versions
        .iter()
        .any(|listed| listed.trim().eq_ignore_ascii_case(version.trim()))
    {
        return true;
    }

    let parsed = match parse_version_flexible(version) {
        Some(parsed) => parsed,
        None => return false,
    };

    versions
        .iter()
        .filter_map(|listed| parse_version_flexible(listed))
        .any(|listed| listed == parsed)
}

/// Whether a single range covers this version.
///
/// `GIT` ranges are commit-based and carry no version to compare, so they are
/// skipped rather than guessed at. `SEMVER` and `ECOSYSTEM` are both evaluated
/// through [`parse_version_flexible`], which is what lets a Debian
/// `2:8.1.0875-5ubuntu2` be placed on a range at all.
fn range_contains(range: &Range, version: &Version) -> bool {
    if range.kind.eq_ignore_ascii_case("GIT") {
        return false;
    }

    intervals(&range.events)
        .iter()
        .any(|interval| interval.contains(version))
}

/// Normalise a range's events into intervals.
///
/// The events are sorted by version before they are walked, so a record whose
/// events arrive out of order — which OSV's schema permits and real data
/// contains — produces the same intervals as a sorted one. An `introduced`
/// with no terminator stays open to infinity; a terminator with no preceding
/// `introduced` starts from zero, which is what "affected from the beginning"
/// means.
fn intervals(events: &[Event]) -> Vec<Interval> {
    let mut points: Vec<(Version, EventKind)> = Vec::new();

    for event in events {
        if let Some(raw) = event.introduced.as_deref() {
            // OSV spells "from the very beginning" as `introduced: "0"`.
            let version = parse_version_flexible(raw).unwrap_or_else(zero_version);
            points.push((version, EventKind::Introduced));
        }
        if let Some(raw) = event.fixed.as_deref() {
            if let Some(version) = parse_version_flexible(raw) {
                points.push((version, EventKind::Fixed));
            }
        }
        if let Some(raw) = event.last_affected.as_deref() {
            if let Some(version) = parse_version_flexible(raw) {
                points.push((version, EventKind::LastAffected));
            }
        }
        // `limit` bounds the range the same way `fixed` does: everything at or
        // above it is outside this range.
        if let Some(raw) = event.limit.as_deref() {
            if let Some(version) = parse_version_flexible(raw) {
                points.push((version, EventKind::Fixed));
            }
        }
    }

    if points.is_empty() {
        return Vec::new();
    }

    // Sort by version, and at equal versions let `introduced` come first so a
    // `fixed` at the same version closes the interval it opened.
    points.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.order().cmp(&b.1.order())));

    let mut out = Vec::new();
    let mut open: Option<Version> = None;

    for (version, kind) in points {
        match kind {
            EventKind::Introduced => {
                // Back-to-back `introduced` events: the earlier one is still
                // open and still unbounded, so it already covers this one.
                if open.is_none() {
                    open = Some(version);
                }
            }
            EventKind::Fixed => {
                let from = open.take().unwrap_or_else(zero_version);
                out.push(Interval {
                    from,
                    to: Bound::Before(version),
                });
            }
            EventKind::LastAffected => {
                let from = open.take().unwrap_or_else(zero_version);
                out.push(Interval {
                    from,
                    to: Bound::Through(version),
                });
            }
        }
    }

    if let Some(from) = open {
        out.push(Interval {
            from,
            to: Bound::Open,
        });
    }

    out
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum EventKind {
    Introduced,
    Fixed,
    LastAffected,
}

impl EventKind {
    /// Tie-break order at an identical version.
    fn order(self) -> u8 {
        match self {
            EventKind::Introduced => 0,
            EventKind::Fixed => 1,
            EventKind::LastAffected => 2,
        }
    }
}

fn zero_version() -> Version {
    Version::new(0, 0, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::osv::AffectedPackage;

    fn event(introduced: Option<&str>, fixed: Option<&str>, last: Option<&str>) -> Event {
        Event {
            introduced: introduced.map(String::from),
            fixed: fixed.map(String::from),
            last_affected: last.map(String::from),
            limit: None,
        }
    }

    fn semver_range(events: Vec<Event>) -> Range {
        Range {
            kind: "SEMVER".to_string(),
            events,
        }
    }

    fn entry(ranges: Vec<Range>, versions: &[&str]) -> Affected {
        Affected {
            package: Some(AffectedPackage {
                ecosystem: "crates.io".to_string(),
                name: "serde".to_string(),
                purl: None,
            }),
            ranges,
            versions: versions.iter().map(|v| v.to_string()).collect(),
            severity: Vec::new(),
            database_specific: None,
        }
    }

    fn hit(entry: &Affected, version: &str) -> bool {
        entry_affects(entry, "crates.io", "serde", version)
    }

    #[test]
    fn test_introduced_only_is_half_open() {
        let affected = entry(
            vec![semver_range(vec![event(Some("1.2.0"), None, None)])],
            &[],
        );
        assert!(!hit(&affected, "1.1.9"));
        assert!(hit(&affected, "1.2.0"));
        assert!(hit(&affected, "9.9.9"));
    }

    #[test]
    fn test_introduced_and_fixed_excludes_the_fix() {
        let affected = entry(
            vec![semver_range(vec![
                event(Some("1.2.0"), None, None),
                event(None, Some("1.2.5"), None),
            ])],
            &[],
        );
        assert!(!hit(&affected, "1.1.0"));
        assert!(hit(&affected, "1.2.0"));
        assert!(hit(&affected, "1.2.4"));
        assert!(!hit(&affected, "1.2.5"));
        assert!(!hit(&affected, "1.3.0"));
    }

    #[test]
    fn test_last_affected_includes_its_own_version() {
        let affected = entry(
            vec![semver_range(vec![
                event(Some("2.0.0"), None, None),
                event(None, None, Some("2.4.0")),
            ])],
            &[],
        );
        assert!(hit(&affected, "2.4.0"));
        assert!(!hit(&affected, "2.4.1"));
    }

    #[test]
    fn test_events_in_reverse_order_still_bound_the_interval() {
        let affected = entry(
            vec![semver_range(vec![
                event(None, Some("1.2.5"), None),
                event(Some("1.2.0"), None, None),
            ])],
            &[],
        );
        assert!(hit(&affected, "1.2.1"));
        assert!(!hit(&affected, "1.2.5"));
        assert!(!hit(&affected, "1.0.0"));
    }

    #[test]
    fn test_fixed_with_no_introduced_starts_from_zero() {
        let affected = entry(
            vec![semver_range(vec![event(None, Some("1.0.0"), None)])],
            &[],
        );
        assert!(hit(&affected, "0.9.0"));
        assert!(!hit(&affected, "1.0.0"));
    }

    #[test]
    fn test_introduced_zero_means_from_the_beginning() {
        let affected = entry(
            vec![semver_range(vec![
                event(Some("0"), None, None),
                event(None, Some("3.0.0"), None),
            ])],
            &[],
        );
        assert!(hit(&affected, "0.0.1"));
        assert!(hit(&affected, "2.9.9"));
        assert!(!hit(&affected, "3.0.0"));
    }

    #[test]
    fn test_two_disjoint_intervals_in_one_range() {
        let affected = entry(
            vec![semver_range(vec![
                event(Some("1.0.0"), None, None),
                event(None, Some("1.5.0"), None),
                event(Some("2.0.0"), None, None),
                event(None, Some("2.1.0"), None),
            ])],
            &[],
        );
        assert!(hit(&affected, "1.4.0"));
        assert!(!hit(&affected, "1.6.0"));
        assert!(hit(&affected, "2.0.5"));
        assert!(!hit(&affected, "2.1.0"));
    }

    #[test]
    fn test_explicit_versions_match_without_any_range() {
        let affected = entry(Vec::new(), &["1.0.1", "1.0.3"]);
        assert!(hit(&affected, "1.0.1"));
        assert!(hit(&affected, "1.0.3"));
        assert!(!hit(&affected, "1.0.2"));
    }

    #[test]
    fn test_explicit_versions_match_after_normalisation() {
        let affected = entry(Vec::new(), &["1.21-76"]);
        assert!(hit(&affected, "1.21"));
        assert!(hit(&affected, "2:1.21-76"));
    }

    #[test]
    fn test_ecosystem_range_type_is_matched_like_semver() {
        let affected = entry(
            vec![Range {
                kind: "ECOSYSTEM".to_string(),
                events: vec![
                    event(Some("8.1.0"), None, None),
                    event(None, Some("8.2.0"), None),
                ],
            }],
            &[],
        );
        assert!(hit(&affected, "8.1.0875"));
        assert!(!hit(&affected, "8.2.0"));
    }

    #[test]
    fn test_git_ranges_are_skipped() {
        let affected = entry(
            vec![Range {
                kind: "GIT".to_string(),
                events: vec![event(Some("0"), None, None)],
            }],
            &[],
        );
        assert!(!hit(&affected, "1.0.0"));
    }

    #[test]
    fn test_debian_style_version_is_normalised_onto_a_range() {
        let affected = Affected {
            package: Some(AffectedPackage {
                ecosystem: "Debian:11".to_string(),
                name: "vim".to_string(),
                purl: None,
            }),
            ranges: vec![Range {
                kind: "ECOSYSTEM".to_string(),
                events: vec![
                    event(Some("0"), None, None),
                    event(None, Some("8.1.1000"), None),
                ],
            }],
            versions: Vec::new(),
            severity: Vec::new(),
            database_specific: None,
        };
        assert!(entry_affects(
            &affected,
            "Debian",
            "vim",
            "2:8.1.0875-5ubuntu2"
        ));
        assert!(!entry_affects(&affected, "Debian", "vim", "2:8.2.0-1"));
    }

    #[test]
    fn test_four_part_version_is_narrowed_to_three() {
        let affected = entry(
            vec![semver_range(vec![
                event(Some("14.1.0"), None, None),
                event(None, Some("14.2.0"), None),
            ])],
            &[],
        );
        assert!(hit(&affected, "14.1.0.0"));
    }

    #[test]
    fn test_entry_for_another_package_never_matches() {
        let affected = entry(
            vec![semver_range(vec![event(Some("0"), None, None)])],
            &["1.0.0"],
        );
        assert!(!entry_affects(&affected, "crates.io", "other", "1.0.0"));
        assert!(!entry_affects(&affected, "NuGet", "serde", "1.0.0"));
    }

    #[test]
    fn test_entry_without_a_package_block_is_taken_at_face_value() {
        let affected = Affected {
            package: None,
            ranges: vec![semver_range(vec![event(Some("0"), None, None)])],
            versions: Vec::new(),
            severity: Vec::new(),
            database_specific: None,
        };
        assert!(entry_affects(&affected, "GitHub", "owner/repo", "1.0.0"));
    }

    #[test]
    fn test_name_comparison_ignores_case() {
        let affected = entry(vec![semver_range(vec![event(Some("0"), None, None)])], &[]);
        assert!(entry_affects(&affected, "crates.io", "SERDE", "1.0.0"));
    }

    #[test]
    fn test_unparseable_version_falls_back_to_the_explicit_list() {
        let affected = entry(
            vec![semver_range(vec![event(Some("0"), None, None)])],
            &["nightly-2024-01-01"],
        );
        assert!(hit(&affected, "nightly-2024-01-01"));
        assert!(!hit(&affected, "nightly-2024-02-02"));
    }

    #[test]
    fn test_limit_bounds_a_range_like_fixed() {
        let affected = entry(
            vec![Range {
                kind: "SEMVER".to_string(),
                events: vec![
                    Event {
                        introduced: Some("1.0.0".to_string()),
                        ..Default::default()
                    },
                    Event {
                        limit: Some("2.0.0".to_string()),
                        ..Default::default()
                    },
                ],
            }],
            &[],
        );
        assert!(hit(&affected, "1.9.0"));
        assert!(!hit(&affected, "2.0.0"));
    }

    #[test]
    fn test_affects_scans_every_entry() {
        let mismatched = entry(
            vec![semver_range(vec![event(Some("9.0.0"), None, None)])],
            &[],
        );
        let matching = entry(
            vec![semver_range(vec![event(Some("1.0.0"), None, None)])],
            &[],
        );
        assert!(affects(
            &[mismatched, matching],
            "crates.io",
            "serde",
            "1.5.0"
        ));
    }

    #[test]
    fn test_empty_range_events_match_nothing() {
        let affected = entry(vec![semver_range(Vec::new())], &[]);
        assert!(!hit(&affected, "1.0.0"));
    }

    #[test]
    fn test_entry_covers_version_ignores_the_package_name() {
        let affected = entry(
            vec![semver_range(vec![
                event(Some("1.0.0"), None, None),
                event(None, Some("2.0.0"), None),
            ])],
            &[],
        );
        // Wrong ecosystem and name, but the version is inside the range.
        assert!(!entry_affects(&affected, "npm", "other", "1.5.0"));
        assert!(entry_covers_version(&affected, "1.5.0"));
        assert!(!entry_covers_version(&affected, "2.0.0"));
    }

    #[test]
    fn test_entry_has_no_version_bound() {
        let unbounded = entry(Vec::new(), &[]);
        assert!(entry_has_no_version_bound(&unbounded));

        let bounded = entry(vec![semver_range(vec![event(Some("0"), None, None)])], &[]);
        assert!(!entry_has_no_version_bound(&bounded));

        let listed = entry(Vec::new(), &["1.0.0"]);
        assert!(!entry_has_no_version_bound(&listed));
    }

    #[test]
    fn test_ecosystem_root_strips_the_release_suffix() {
        assert_eq!(ecosystem_root("Debian:11"), "Debian");
        assert_eq!(ecosystem_root("crates.io"), "crates.io");
    }

    #[test]
    fn test_prerelease_of_a_fixed_version_is_inside_the_range() {
        // The rc precedes the fix, so it is inside `[0, 1.2.3)`. Before the
        // precise parser it collapsed to 1.2.3 and was reported clean.
        let affected = entry(
            vec![semver_range(vec![
                event(Some("0"), None, None),
                event(None, Some("1.2.3"), None),
            ])],
            &[],
        );
        assert!(hit(&affected, "1.2.3-rc1"));
        assert!(!hit(&affected, "1.2.3"));
        // Build metadata carries no ordering, so it still reads as 1.2.3.
        assert!(!hit(&affected, "1.2.3+build"));
        assert!(hit(&affected, "1.2.2+build"));
    }

    #[test]
    fn test_distro_revisions_keep_the_lossy_parser() {
        // `1.2.3-7` parses as semver with a `7` pre-release, which would put a
        // patched Debian package inside a range fixed at `1.2.3-6`.
        let mut affected = entry(
            vec![Range {
                kind: "ECOSYSTEM".to_string(),
                events: vec![
                    event(Some("0"), None, None),
                    event(None, Some("1.2.3-6"), None),
                ],
            }],
            &[],
        );
        affected.package = Some(AffectedPackage {
            ecosystem: "Debian:12".to_string(),
            name: "vim".to_string(),
            purl: None,
        });
        assert!(!entry_affects(&affected, "Debian", "vim", "1.2.3-7"));
        assert!(!entry_affects(&affected, "Debian", "vim", "2:1.2.3-7"));
        assert!(entry_affects(&affected, "Debian", "vim", "1.2.2-9"));
        // The declared-alias path has no query ecosystem; the entry's own
        // ecosystem makes the same call.
        assert!(!entry_covers_version(&affected, "1.2.3-7"));
    }

    #[test]
    fn test_parse_advisory_version_keeps_only_semver_prereleases() {
        let rc = parse_advisory_version(" 1.2.3-rc1 ").unwrap();
        assert_eq!(rc.pre.as_str(), "rc1");
        let built = parse_advisory_version("1.2.3-rc.1+abc").unwrap();
        assert_eq!(built.build.as_str(), "abc");
        // Distro revisions on a three-part version *do* parse as semver
        // pre-releases. This is why distro ecosystems never reach this parser.
        let fedora = parse_advisory_version("8.2.2637-20.fc36").unwrap();
        assert_eq!(fedora.pre.as_str(), "20.fc36");
        // Everything else matches the lossy parser exactly.
        for raw in [
            "1.2.3",
            "1.2.3+build",
            "2:8.1.0875-5ubuntu2",
            "1.21-76",
            "1.21",
        ] {
            assert_eq!(
                parse_advisory_version(raw),
                parse_version_flexible(raw),
                "{raw}"
            );
        }
    }

    #[test]
    fn test_explicit_list_still_matches_an_rc_of_a_listed_version() {
        // A precise comparison would lose this hit; the list stays lossy so
        // the publisher's own enumeration can only ever add matches.
        let affected = entry(Vec::new(), &["1.2.3"]);
        assert!(hit(&affected, "1.2.3-rc1"));
    }
}
