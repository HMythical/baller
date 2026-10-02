//! Turning an advisory's severity into a decision.
//!
//! OSV publishes severity as a CVSS *vector string*, not a number, so the first
//! job here is computing a base score from the vector — CVSS v3.x and v2 are
//! both implemented from their published formulas, and a bare number is taken
//! as already-scored. The second job is the policy: a CVSS 0–10 becomes a
//! Referee Risk Index on a 0–5 scale (`cvss / 2`), and two configurable
//! thresholds split that index into pass, warn and block.
//!
//! The one rule that is not arithmetic: **an advisory with no severity anyone
//! published is never scored as safe.** It cannot be given a number, so it
//! warns, and it never blocks on a figure nobody stated.

use serde_json::Value;

use crate::error::error::BallError;

/// The highest possible Referee Risk Index (CVSS 10.0 halved).
pub const MAX_RISK: f32 = 5.0;

/// What the gate does about a package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    /// Below `warn_at` — install silently
    Pass,
    /// Between the thresholds, or scoreless — print the advisory, install anyway
    Warn,
    /// At or above `block_at` — abort the whole plan
    Block,
}

impl Band {
    pub fn label(self) -> &'static str {
        match self {
            Band::Pass => "pass",
            Band::Warn => "warn",
            Band::Block => "block",
        }
    }
}

/// The two thresholds that split the risk index into [`Band`]s.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RefereeThresholds {
    pub warn_at: f32,
    pub block_at: f32,
}

impl Default for RefereeThresholds {
    fn default() -> Self {
        // CVSS 5.0 (Medium) warns, CVSS 8.0 (High) blocks.
        Self {
            warn_at: 2.5,
            block_at: 4.0,
        }
    }
}

impl RefereeThresholds {
    /// Reject a policy that cannot mean anything: `0 <= warn_at < block_at <= 5`.
    pub fn validate(&self) -> Result<(), BallError> {
        if !self.warn_at.is_finite() || !self.block_at.is_finite() {
            return Err(BallError::InvalidConfig(
                "referee thresholds must be numbers".to_string(),
            ));
        }
        if self.warn_at < 0.0 || self.block_at < 0.0 {
            return Err(BallError::InvalidConfig(format!(
                "referee thresholds cannot be negative (warn_at = {}, block_at = {})",
                self.warn_at, self.block_at
            )));
        }
        if self.block_at > MAX_RISK || self.warn_at > MAX_RISK {
            return Err(BallError::InvalidConfig(format!(
                "referee thresholds must be <= {} on the 0-5 risk scale (warn_at = {}, block_at = {})",
                MAX_RISK, self.warn_at, self.block_at
            )));
        }
        if self.warn_at >= self.block_at {
            return Err(BallError::InvalidConfig(format!(
                "referee warn_at ({}) must be below block_at ({})",
                self.warn_at, self.block_at
            )));
        }
        Ok(())
    }
}

/// The Referee Risk Index for a CVSS base score: the 0–10 scale, halved.
pub fn risk_index(cvss: f32) -> f32 {
    (cvss / 2.0).clamp(0.0, MAX_RISK)
}

/// Which band a package's risk falls in.
///
/// `None` is a matched advisory carrying no severity at all. It warns: absence
/// of a score is not evidence of safety, but it is also not grounds to block on
/// a number that was never published.
pub fn classify(risk: Option<f32>, thresholds: &RefereeThresholds) -> Band {
    match risk {
        None => Band::Warn,
        Some(risk) if risk >= thresholds.block_at => Band::Block,
        Some(risk) if risk >= thresholds.warn_at => Band::Warn,
        Some(_) => Band::Pass,
    }
}

/// The CVSS base score for an OSV `severity` list, highest first.
///
/// Unreadable entries are skipped rather than failing the whole record: one
/// malformed vector must not hide a second, readable one.
pub fn best_cvss(entries: &[crate::security::osv::Severity]) -> Option<f32> {
    entries
        .iter()
        .filter_map(|entry| cvss_score(&entry.score))
        .fold(None, |best: Option<f32>, score| {
            Some(best.map_or(score, |best| best.max(score)))
        })
}

/// A CVSS base score from whatever the database put in `score`.
///
/// Handles a plain number, a CVSS v3.x vector and a CVSS v2 vector. CVSS v4.0
/// vectors are deliberately not scored here — its formula is a lookup table
/// baller does not carry — so a v4-only advisory falls through to the
/// qualitative rating in `database_specific`.
pub fn cvss_score(score: &str) -> Option<f32> {
    let raw = score.trim();
    if raw.is_empty() {
        return None;
    }

    if let Ok(value) = raw.parse::<f32>() {
        if value.is_finite() && (0.0..=10.0).contains(&value) {
            return Some(value);
        }
        return None;
    }

    let upper = raw.to_ascii_uppercase();
    if upper.starts_with("CVSS:3") {
        return cvss3_base(&upper);
    }
    if upper.starts_with("CVSS:4") {
        return None;
    }
    // A v2 vector has no prefix at all — `AV:N/AC:L/Au:N/C:P/I:P/A:P`.
    if upper.starts_with("AV:") {
        return cvss2_base(&upper);
    }

    None
}

/// The qualitative severity some databases publish instead of a vector.
///
/// Read only when no vector could be scored, and mapped to the midpoint of each
/// CVSS v3 severity band so the number is honest about being a rating rather
/// than a measurement.
pub fn qualitative_cvss(database_specific: Option<&Value>) -> Option<f32> {
    let severity = database_specific?
        .get("severity")
        .and_then(Value::as_str)?
        .trim()
        .to_ascii_uppercase();

    match severity.as_str() {
        "CRITICAL" => Some(9.0),
        "HIGH" => Some(7.5),
        "MODERATE" | "MEDIUM" => Some(5.0),
        "LOW" => Some(3.0),
        _ => None,
    }
}

/// Split a vector string into its `KEY:VALUE` metrics.
fn metric(vector: &str, key: &str) -> Option<String> {
    vector.split('/').find_map(|part| {
        let (name, value) = part.split_once(':')?;
        if name.trim().eq_ignore_ascii_case(key) {
            Some(value.trim().to_string())
        } else {
            None
        }
    })
}

/// CVSS v3.0/v3.1 base score, per the published specification.
fn cvss3_base(vector: &str) -> Option<f32> {
    let scope_changed = match metric(vector, "S")?.as_str() {
        "C" => true,
        "U" => false,
        _ => return None,
    };

    let av = match metric(vector, "AV")?.as_str() {
        "N" => 0.85,
        "A" => 0.62,
        "L" => 0.55,
        "P" => 0.2,
        _ => return None,
    };
    let ac = match metric(vector, "AC")?.as_str() {
        "L" => 0.77,
        "H" => 0.44,
        _ => return None,
    };
    let pr = match (metric(vector, "PR")?.as_str(), scope_changed) {
        ("N", _) => 0.85,
        ("L", false) => 0.62,
        ("L", true) => 0.68,
        ("H", false) => 0.27,
        ("H", true) => 0.5,
        _ => return None,
    };
    let ui = match metric(vector, "UI")?.as_str() {
        "N" => 0.85,
        "R" => 0.62,
        _ => return None,
    };

    let cia = |key: &str| -> Option<f64> {
        match metric(vector, key)?.as_str() {
            "H" => Some(0.56),
            "L" => Some(0.22),
            "N" => Some(0.0),
            _ => None,
        }
    };
    let (c, i, a) = (cia("C")?, cia("I")?, cia("A")?);

    let iss = 1.0 - ((1.0 - c) * (1.0 - i) * (1.0 - a));
    let impact = if scope_changed {
        7.52 * (iss - 0.029) - 3.25 * (iss - 0.02).powi(15)
    } else {
        6.42 * iss
    };

    if impact <= 0.0 {
        return Some(0.0);
    }

    let exploitability = 8.22 * av * ac * pr * ui;
    let raw = if scope_changed {
        (1.08 * (impact + exploitability)).min(10.0)
    } else {
        (impact + exploitability).min(10.0)
    };

    Some(roundup(raw) as f32)
}

/// CVSS v3.1's "round up to one decimal", in the integer form the spec gives
/// so that a value already at a tenth is not pushed to the next one.
fn roundup(value: f64) -> f64 {
    let scaled = (value * 100_000.0).round() as i64;
    if scaled % 10_000 == 0 {
        scaled as f64 / 100_000.0
    } else {
        ((scaled / 10_000) + 1) as f64 / 10.0
    }
}

/// CVSS v2 base score, per the published specification.
fn cvss2_base(vector: &str) -> Option<f32> {
    let av = match metric(vector, "AV")?.as_str() {
        "L" => 0.395,
        "A" => 0.646,
        "N" => 1.0,
        _ => return None,
    };
    let ac = match metric(vector, "AC")?.as_str() {
        "H" => 0.35,
        "M" => 0.61,
        "L" => 0.71,
        _ => return None,
    };
    let au = match metric(vector, "AU")?.as_str() {
        "M" => 0.45,
        "S" => 0.56,
        "N" => 0.704,
        _ => return None,
    };

    let cia = |key: &str| -> Option<f64> {
        match metric(vector, key)?.as_str() {
            "N" => Some(0.0),
            "P" => Some(0.275),
            "C" => Some(0.660),
            _ => None,
        }
    };
    let (c, i, a) = (cia("C")?, cia("I")?, cia("A")?);

    let impact = 10.41 * (1.0 - (1.0 - c) * (1.0 - i) * (1.0 - a));
    let exploitability = 20.0 * av * ac * au;
    let f_impact = if impact == 0.0 { 0.0 } else { 1.176 };
    let score = ((0.6 * impact) + (0.4 * exploitability) - 1.5) * f_impact;

    Some(((score * 10.0).round() / 10.0).clamp(0.0, 10.0) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::osv::Severity;
    use serde_json::json;

    fn severity(kind: &str, score: &str) -> Severity {
        Severity {
            kind: kind.to_string(),
            score: score.to_string(),
        }
    }

    #[test]
    fn test_risk_index_halves_the_cvss_scale() {
        assert_eq!(risk_index(10.0), 5.0);
        assert_eq!(risk_index(7.5), 3.75);
        assert_eq!(risk_index(0.0), 0.0);
    }

    #[test]
    fn test_risk_index_clamps_out_of_range_input() {
        assert_eq!(risk_index(99.0), MAX_RISK);
        assert_eq!(risk_index(-4.0), 0.0);
    }

    #[test]
    fn test_default_thresholds_band_the_documented_way() {
        let t = RefereeThresholds::default();
        assert_eq!(classify(Some(risk_index(4.9)), &t), Band::Pass);
        assert_eq!(classify(Some(risk_index(5.0)), &t), Band::Warn);
        assert_eq!(classify(Some(risk_index(7.9)), &t), Band::Warn);
        assert_eq!(classify(Some(risk_index(8.0)), &t), Band::Block);
        assert_eq!(classify(Some(risk_index(10.0)), &t), Band::Block);
    }

    #[test]
    fn test_scoreless_advisory_warns_and_never_blocks() {
        let t = RefereeThresholds::default();
        assert_eq!(classify(None, &t), Band::Warn);
        let zero_block = RefereeThresholds {
            warn_at: 0.0,
            block_at: 0.1,
        };
        assert_eq!(classify(None, &zero_block), Band::Warn);
    }

    #[test]
    fn test_warn_at_zero_warns_on_anything_scored() {
        let t = RefereeThresholds {
            warn_at: 0.0,
            block_at: 4.0,
        };
        assert_eq!(classify(Some(0.0), &t), Band::Warn);
        assert_eq!(classify(Some(0.5), &t), Band::Warn);
    }

    #[test]
    fn test_block_at_zero_blocks_on_any_advisory() {
        let t = RefereeThresholds {
            warn_at: 0.0,
            block_at: 0.0,
        };
        // 0/0 is rejected by validate, but classify still has to be total.
        assert_eq!(classify(Some(0.0), &t), Band::Block);
    }

    #[test]
    fn test_validate_accepts_the_defaults() {
        assert!(RefereeThresholds::default().validate().is_ok());
    }

    #[test]
    fn test_validate_rejects_warn_at_or_above_block() {
        let t = RefereeThresholds {
            warn_at: 4.0,
            block_at: 4.0,
        };
        let err = t.validate().unwrap_err();
        assert!(format!("{}", err).contains("must be below block_at"));

        let t = RefereeThresholds {
            warn_at: 4.5,
            block_at: 4.0,
        };
        assert!(t.validate().is_err());
    }

    #[test]
    fn test_validate_rejects_out_of_scale_values() {
        assert!(RefereeThresholds {
            warn_at: -0.1,
            block_at: 4.0
        }
        .validate()
        .is_err());
        assert!(RefereeThresholds {
            warn_at: 2.5,
            block_at: 5.1
        }
        .validate()
        .is_err());
        assert!(RefereeThresholds {
            warn_at: f32::NAN,
            block_at: 4.0
        }
        .validate()
        .is_err());
    }

    #[test]
    fn test_cvss3_vectors_score_to_their_published_values() {
        // Published CVSS v3.1 examples.
        let cases = [
            ("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H", 9.8),
            ("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:N/A:N", 7.5),
            ("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:N/I:N/A:H", 7.5),
            ("CVSS:3.1/AV:L/AC:L/PR:L/UI:N/S:U/C:H/I:H/A:H", 7.8),
            ("CVSS:3.1/AV:N/AC:L/PR:N/UI:R/S:C/C:L/I:L/A:N", 6.1),
            ("CVSS:3.0/AV:N/AC:H/PR:H/UI:R/S:U/C:N/I:N/A:N", 0.0),
            ("CVSS:3.1/AV:L/AC:H/PR:H/UI:R/S:U/C:L/I:N/A:N", 1.8),
        ];
        for (vector, expected) in cases {
            let score = cvss_score(vector).unwrap_or_else(|| panic!("no score for {}", vector));
            assert!(
                (score - expected).abs() < 0.05,
                "{} scored {} (expected {})",
                vector,
                score,
                expected
            );
        }
    }

    #[test]
    fn test_cvss3_vector_is_case_insensitive() {
        let upper = cvss_score("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H").unwrap();
        let lower = cvss_score("cvss:3.1/av:n/ac:l/pr:n/ui:n/s:u/c:h/i:h/a:h").unwrap();
        assert_eq!(upper, lower);
    }

    #[test]
    fn test_cvss3_vector_with_trailing_temporal_metrics_still_scores() {
        let score = cvss_score("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H/E:P/RL:O").unwrap();
        assert!((score - 9.8).abs() < 0.05);
    }

    #[test]
    fn test_cvss2_vectors_score_to_their_published_values() {
        let cases = [
            ("AV:N/AC:L/Au:N/C:P/I:P/A:P", 7.5),
            ("AV:N/AC:L/Au:N/C:C/I:C/A:C", 10.0),
            ("AV:L/AC:H/Au:N/C:N/I:N/A:P", 1.2),
        ];
        for (vector, expected) in cases {
            let score = cvss_score(vector).unwrap_or_else(|| panic!("no score for {}", vector));
            assert!(
                (score - expected).abs() < 0.05,
                "{} scored {} (expected {})",
                vector,
                score,
                expected
            );
        }
    }

    #[test]
    fn test_bare_number_scores_are_taken_as_given() {
        assert_eq!(cvss_score("7.5"), Some(7.5));
        assert_eq!(cvss_score(" 9 "), Some(9.0));
        assert_eq!(cvss_score("10.0"), Some(10.0));
    }

    #[test]
    fn test_out_of_range_numbers_are_rejected() {
        assert_eq!(cvss_score("11.0"), None);
        assert_eq!(cvss_score("-1"), None);
    }

    #[test]
    fn test_unscoreable_inputs_return_none() {
        assert_eq!(cvss_score(""), None);
        assert_eq!(cvss_score("   "), None);
        assert_eq!(cvss_score("HIGH"), None);
        assert_eq!(
            cvss_score("CVSS:3.1/AV:Z/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H"),
            None
        );
        assert_eq!(cvss_score("CVSS:3.1/AV:N/AC:L"), None);
    }

    #[test]
    fn test_cvss4_vectors_are_left_unscored() {
        assert_eq!(
            cvss_score("CVSS:4.0/AV:N/AC:L/AT:N/PR:N/UI:N/VC:H/VI:H/VA:H/SC:N/SI:N/SA:N"),
            None
        );
    }

    #[test]
    fn test_best_cvss_takes_the_highest_readable_entry() {
        let entries = [
            severity("CVSS_V3", "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:N/A:N"),
            severity("CVSS_V3", "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H"),
            severity("CVSS_V4", "CVSS:4.0/AV:N/AC:L/AT:N/PR:N/UI:N"),
        ];
        let score = best_cvss(&entries).unwrap();
        assert!((score - 9.8).abs() < 0.05);
    }

    #[test]
    fn test_best_cvss_is_none_when_nothing_is_readable() {
        assert_eq!(best_cvss(&[]), None);
        assert_eq!(best_cvss(&[severity("CVSS_V4", "CVSS:4.0/AV:N")]), None);
    }

    #[test]
    fn test_qualitative_severity_is_the_last_resort() {
        let db = json!({ "severity": "CRITICAL" });
        assert_eq!(qualitative_cvss(Some(&db)), Some(9.0));
        let db = json!({ "severity": "moderate" });
        assert_eq!(qualitative_cvss(Some(&db)), Some(5.0));
        let db = json!({ "severity": "unhelpful" });
        assert_eq!(qualitative_cvss(Some(&db)), None);
        assert_eq!(qualitative_cvss(None), None);
        assert_eq!(qualitative_cvss(Some(&json!({}))), None);
    }

    #[test]
    fn test_roundup_never_promotes_an_exact_tenth() {
        assert!((roundup(4.0) - 4.0).abs() < 1e-9);
        assert!((roundup(4.02) - 4.1).abs() < 1e-9);
    }

    #[test]
    fn test_band_labels() {
        assert_eq!(Band::Pass.label(), "pass");
        assert_eq!(Band::Warn.label(), "warn");
        assert_eq!(Band::Block.label(), "block");
    }
}
