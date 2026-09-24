//! `baller referee check` — advisory data only (Phase A).
//!
//! Exactly the path `audit --no-scan` takes, as its own verb: no extracted
//! tree is walked or re-scanned.

use super::audit::{execute_audit, AuditOptions};
use super::FailOn;
use crate::context::AppContext;
use crate::error::error::BallError;

pub fn execute_check(
    ctx: &AppContext,
    package_names: &[String],
    refresh: bool,
    fail_on: Option<FailOn>,
) -> Result<(), BallError> {
    execute_audit(
        ctx,
        &AuditOptions {
            package_names: package_names.to_vec(),
            refresh,
            no_scan: true,
            fail_on,
            format: None,
            out: None,
        },
    )
}
