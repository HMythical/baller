use crate::cli::help::execute_command_help;
use crate::cli::version::execute_command_version;
use crate::commands::build::{execute_build, BuildOptions};
use crate::commands::draft::{execute_draft, DraftOptions};
use crate::commands::eject::{execute_eject, EjectOptions};
use crate::commands::external::execute_external;
use crate::commands::freeze::{execute_freeze, FreezeMode, FreezeOptions};
use crate::commands::inject::execute_inject;
use crate::commands::referee::{
    execute_referee, AuditFormat, AuditOptions, CacheAction, FailOn, RefereeCommand, SbomFormat,
};
use crate::commands::roster::{execute_roster, RosterOptions};
use crate::commands::substitute::{execute_substitute, SubstituteOptions};
use crate::commands::sweep::{execute_sweep, SweepOptions};
use crate::commands::update::{execute_update, UpdateOptions};
use crate::context::{AppContext, GlobalFlags};
use crate::core::injected::resolve_baller_dir;
use crate::core::registry::RegistrySource;
use crate::error::error::BallError;
use clap::error::ErrorKind;
use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
#[command(name = "baller")]
#[command(about = "B.A.L.L.E.R - The Binary Allocation & Library Launch Environment in Rust", long_about = None)]
#[command(version)]
#[command(after_help = "Run 'baller help' to also list injected commands.")]
// Baller ships its own 'help' subcommand: clap's built-in one cannot see
// injected commands.
#[command(disable_help_subcommand = true)]
pub struct BallerCommand {
    #[command(subcommand)]
    pub command: CommandTypes,

    /// Skip confirmation prompts
    #[arg(short = 'y', long, global = true)]
    pub yes: bool,

    /// Suppress progress bars and step-by-step output
    #[arg(short = 'q', long, global = true)]
    pub quiet: bool,

    /// Skip every pre/post install, eject and update hook
    #[arg(long = "no-hooks", global = true)]
    pub no_hooks: bool,

    /// Disable colored output
    #[arg(long = "no-color", global = true)]
    pub no_color: bool,

    /// Use an alternate baller directory (config, db, cache and hooks)
    #[arg(long, global = true, value_name = "DIR")]
    pub config: Option<String>,

    /// Emit machine-readable JSON instead of formatted text
    #[arg(long, global = true)]
    pub json: bool,

    /// Increase output detail
    #[arg(short = 'v', long, global = true)]
    pub verbose: bool,

    /// Skip the Referee security checks for this run
    #[arg(long = "no-referee", global = true)]
    pub no_referee: bool,
}

/// A registry source named on the command line
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum SourceArg {
    Github,
    Baller,
    Chocolatey,
    System,
    Cargo,
}

impl SourceArg {
    pub fn to_registry_source(self) -> RegistrySource {
        match self {
            SourceArg::Github => RegistrySource::GitHub,
            SourceArg::Baller => RegistrySource::BallerRegistry,
            SourceArg::Chocolatey => RegistrySource::Chocolatey,
            SourceArg::System => RegistrySource::System,
            SourceArg::Cargo => RegistrySource::Cargo,
        }
    }
}

#[derive(Subcommand, Debug)]
pub enum CommandTypes {
    /// Drafts (Installs) a new player onto your team
    Draft {
        package_name: String,
        /// Pin an exact version (GitHub and Chocolatey sources only)
        #[arg(long, value_name = "VERSION")]
        version: Option<String>,
        /// Resolve from one registry instead of the configured chain
        #[arg(long, value_enum, value_name = "SOURCE")]
        source: Option<SourceArg>,
        /// Install the package alone, ignoring its dependencies
        #[arg(long = "no-deps")]
        no_deps: bool,
        /// Show what would be installed without touching anything
        #[arg(long = "dry-run")]
        dry_run: bool,
        /// Reinstall even when the package is already on the roster
        #[arg(short, long)]
        force: bool,
    },
    /// Ejects (Uninstalls) a player from your team
    #[command(arg_required_else_help = false)]
    Eject {
        package_name: String,
        /// Eject even when the package is frozen
        #[arg(short, long)]
        force: bool,
        /// Also delete the cached download archive
        #[arg(long)]
        purge: bool,
        /// Leave orphaned dependencies installed
        #[arg(long = "no-orphans")]
        no_orphans: bool,
        /// Drop the roster entry but leave the linked binary in place
        #[arg(long = "keep-bin")]
        keep_bin: bool,
    },
    /// Freezes (Pins) a player so they cannot be substituted or updated
    Freeze {
        package_name: Option<String>,
        /// Freeze explicitly instead of toggling
        #[arg(long, conflicts_with = "thaw")]
        freeze: bool,
        /// Thaw explicitly instead of toggling
        #[arg(long)]
        thaw: bool,
        /// Apply to every installed package
        #[arg(long)]
        all: bool,
        /// List the frozen packages
        #[arg(long)]
        list: bool,
    },
    /// Rosters (Lists) active players on your team or searches for one
    Roster {
        package_name: Option<String>,
        /// Only show frozen packages
        #[arg(long)]
        frozen: bool,
        /// Only show packages installed from this source
        #[arg(long, value_enum, value_name = "SOURCE")]
        source: Option<SourceArg>,
        /// Check registries for newer versions without updating
        #[arg(long)]
        outdated: bool,
        /// Search registries instead of the local roster
        #[arg(long)]
        remote: bool,
    },
    /// Substitutes (Swaps) a current player for a new one cleanly
    Substitute {
        old_package: String,
        new_package: String,
        /// Install the new package but leave the old one installed
        #[arg(long = "keep-old")]
        keep_old: bool,
        /// Show what would change without touching anything
        #[arg(long = "dry-run")]
        dry_run: bool,
        /// Install the new package alone, ignoring its dependencies
        #[arg(long = "no-deps")]
        no_deps: bool,
    },
    /// Sweeps (Cleans) the arena of leftover caching debris
    #[command(arg_required_else_help = false)]
    Sweep {
        /// Also delete extracted packages, not just downloaded archives
        #[arg(long, alias = "purge-extracted")]
        all: bool,
        /// Report what would be swept without deleting
        #[arg(long = "dry-run")]
        dry_run: bool,
        /// Only sweep when the cache is larger than this (e.g. 50MB)
        #[arg(long, value_name = "SIZE")]
        threshold: Option<String>,
    },
    /// Updates all active packages on the team
    Update {
        /// Update only these packages (default: everything)
        packages: Vec<String>,
        /// Report stale packages without updating them
        #[arg(long, alias = "dry-run")]
        check: bool,
        /// Update frozen packages too
        #[arg(long = "include-frozen")]
        include_frozen: bool,
    },
    /// Builds a package natively from a local manifest
    Build {
        path: String,
        /// Parse and validate the manifest without installing
        #[arg(long = "dry-run")]
        dry_run: bool,
        /// Ignore the manifest's declared dependencies
        #[arg(long = "no-deps")]
        no_deps: bool,
        /// Link the binary into this directory instead of the default
        #[arg(long = "install-dir", value_name = "DIR")]
        install_dir: Option<String>,
        /// Build over an existing installation of the same package
        #[arg(short, long)]
        force: bool,
        /// Override the manifest's source before resolving
        #[arg(long, value_enum, value_name = "SOURCE")]
        source: Option<SourceArg>,
    },
    /// Injects a custom command described by a .ball file
    Inject { path: String },
    /// Referees (Audits) the roster against public vulnerability data
    #[command(arg_required_else_help = false, args_conflicts_with_subcommands = true)]
    Referee(RefereeArgs),
    /// Prints every available command, or details for one of them
    Help {
        /// Built-in or injected command to describe
        command: Option<String>,
    },
    /// Prints baller's version
    Version,
    /// Any unknown subcommand: dispatched to an injected command, if one matches
    #[command(external_subcommand)]
    External(Vec<String>),
}

/// `baller referee`: a command group whose bare form is `audit`.
///
/// `package_name`, `--refresh` and `--no-scan` keep `baller referee [PACKAGE]`
/// working exactly as it did before the group existed. They conflict with a
/// subcommand, so `baller referee fd` is an audit of `fd` while
/// `baller referee audit fd` names the verb explicitly.
#[derive(Args, Debug)]
pub struct RefereeArgs {
    #[command(subcommand)]
    pub command: Option<RefereeSub>,
    /// Package to audit; omit to audit the whole roster
    pub package_name: Option<String>,
    /// Re-query the advisory service instead of reusing cached verdicts
    #[arg(long)]
    pub refresh: bool,
    /// Skip the artifact re-scan and only check advisory data
    #[arg(long = "no-scan")]
    pub no_scan: bool,
}

#[derive(Subcommand, Debug)]
pub enum RefereeSub {
    /// Re-checks advisory data and re-scans installed artifacts (the default)
    Audit {
        /// Packages to audit; omit to audit the whole roster
        package_names: Vec<String>,
        /// Re-query the advisory service instead of reusing cached verdicts
        #[arg(long)]
        refresh: bool,
        /// Skip the artifact re-scan and only check advisory data
        #[arg(long = "no-scan")]
        no_scan: bool,
        /// Exit non-zero when any package reaches this band
        #[arg(long = "fail-on", value_enum, value_name = "BAND")]
        fail_on: Option<FailOn>,
        /// Render the report as json, markdown or sarif
        #[arg(long, value_enum, value_name = "FORMAT")]
        format: Option<AuditFormat>,
        /// Write the formatted report to this file instead of stdout
        #[arg(long, value_name = "FILE", requires = "format")]
        out: Option<String>,
    },
    /// Checks advisory data only (Phase A); nothing is re-scanned
    Check {
        /// Packages to check; omit to check the whole roster
        package_names: Vec<String>,
        /// Re-query the advisory service instead of reusing cached verdicts
        #[arg(long)]
        refresh: bool,
        /// Exit non-zero when any package reaches this band
        #[arg(long = "fail-on", value_enum, value_name = "BAND")]
        fail_on: Option<FailOn>,
    },
    /// Re-scans installed artifacts only (Phase B); no advisory lookup
    Scan {
        /// Packages to scan; omit to scan the whole roster
        package_names: Vec<String>,
    },
    /// Shows, clears or prunes the verdict cache
    Cache {
        /// Show cached verdicts per ecosystem (the default)
        #[arg(long, conflicts_with_all = ["clear", "prune"])]
        status: bool,
        /// Drop every cached verdict
        #[arg(long, conflicts_with = "prune")]
        clear: bool,
        /// Drop verdicts computed more than DAYS days ago
        #[arg(long, value_name = "DAYS")]
        prune: Option<u32>,
    },
    /// Prints the [referee] settings in effect
    Config,
    /// Writes a CycloneDX SBOM of the roster
    Sbom {
        /// Write the SBOM to this file instead of stdout
        #[arg(long, value_name = "FILE")]
        out: Option<String>,
        /// SBOM format
        #[arg(
            long,
            value_enum,
            value_name = "FORMAT",
            default_value = "cyclonedx-json"
        )]
        format: SbomFormat,
    },
}

impl RefereeArgs {
    /// What to run: the named subcommand, or `audit` for the bare form.
    pub fn to_command(&self) -> RefereeCommand {
        match &self.command {
            None => RefereeCommand::Audit(AuditOptions {
                package_names: self.package_name.iter().cloned().collect(),
                refresh: self.refresh,
                no_scan: self.no_scan,
                fail_on: None,
                format: None,
                out: None,
            }),
            Some(RefereeSub::Audit {
                package_names,
                refresh,
                no_scan,
                fail_on,
                format,
                out,
            }) => RefereeCommand::Audit(AuditOptions {
                package_names: package_names.clone(),
                refresh: *refresh,
                no_scan: *no_scan,
                fail_on: *fail_on,
                format: *format,
                out: out.clone(),
            }),
            Some(RefereeSub::Check {
                package_names,
                refresh,
                fail_on,
            }) => RefereeCommand::Check {
                package_names: package_names.clone(),
                refresh: *refresh,
                fail_on: *fail_on,
            },
            Some(RefereeSub::Scan { package_names }) => RefereeCommand::Scan {
                package_names: package_names.clone(),
            },
            Some(RefereeSub::Cache { clear, prune, .. }) => RefereeCommand::Cache(match prune {
                Some(days) => CacheAction::Prune(*days),
                None if *clear => CacheAction::Clear,
                None => CacheAction::Status,
            }),
            Some(RefereeSub::Config) => RefereeCommand::Config,
            Some(RefereeSub::Sbom { out, format }) => RefereeCommand::Sbom {
                out: out.clone(),
                format: *format,
            },
        }
    }
}

impl BallerCommand {
    pub fn parse_command() -> Result<Self, BallError> {
        match BallerCommand::try_parse() {
            Ok(cmd) => Ok(cmd),
            // Help and version are requests, not failures: let clap print them
            // on its own stream and exit the way it normally would.
            Err(e) if is_display_request(e.kind()) => e.exit(),
            Err(e) => Err(BallError::InvalidConfig(format!("CLI Error: {}", e))),
        }
    }

    /// The flags every subcommand honors, lifted out for `AppContext`
    pub fn global_flags(&self) -> GlobalFlags {
        GlobalFlags {
            yes: self.yes,
            quiet: self.quiet,
            json: self.json,
            verbose: self.verbose,
            no_referee: self.no_referee,
        }
    }

    pub fn execute(&self, ctx: &AppContext) -> Result<(), BallError> {
        match &self.command {
            CommandTypes::Draft {
                package_name,
                version,
                source,
                no_deps,
                dry_run,
                force,
            } => execute_draft(
                ctx,
                package_name,
                &DraftOptions {
                    version: version.clone(),
                    source: source.map(SourceArg::to_registry_source),
                    no_deps: *no_deps,
                    dry_run: *dry_run,
                    force: *force,
                },
            ),
            CommandTypes::Eject {
                package_name,
                force,
                purge,
                no_orphans,
                keep_bin,
            } => execute_eject(
                ctx,
                package_name,
                &EjectOptions {
                    force: *force,
                    purge: *purge,
                    no_orphans: *no_orphans,
                    keep_bin: *keep_bin,
                },
            ),
            CommandTypes::Freeze {
                package_name,
                freeze,
                thaw,
                all,
                list,
            } => execute_freeze(
                ctx,
                package_name,
                &FreezeOptions {
                    mode: FreezeMode::from_flags(*freeze, *thaw),
                    all: *all,
                    list: *list,
                },
            ),
            CommandTypes::Roster {
                package_name,
                frozen,
                source,
                outdated,
                remote,
            } => execute_roster(
                ctx,
                package_name,
                &RosterOptions {
                    frozen: *frozen,
                    source: source.map(SourceArg::to_registry_source),
                    outdated: *outdated,
                    remote: *remote,
                },
            ),
            CommandTypes::Substitute {
                old_package,
                new_package,
                keep_old,
                dry_run,
                no_deps,
            } => execute_substitute(
                ctx,
                old_package,
                new_package,
                &SubstituteOptions {
                    keep_old: *keep_old,
                    dry_run: *dry_run,
                    no_deps: *no_deps,
                },
            ),
            CommandTypes::Sweep {
                all,
                dry_run,
                threshold,
            } => execute_sweep(
                ctx,
                &SweepOptions {
                    all: *all,
                    dry_run: *dry_run,
                    threshold: threshold.clone(),
                },
            ),
            CommandTypes::Update {
                packages,
                check,
                include_frozen,
            } => execute_update(
                ctx,
                &UpdateOptions {
                    packages: packages.clone(),
                    check: *check,
                    include_frozen: *include_frozen,
                },
            ),
            CommandTypes::Build {
                path,
                dry_run,
                no_deps,
                install_dir,
                force,
                source,
            } => execute_build(
                ctx,
                path,
                &BuildOptions {
                    dry_run: *dry_run,
                    no_deps: *no_deps,
                    install_dir: install_dir.clone(),
                    force: *force,
                    source: source.map(SourceArg::to_registry_source),
                },
            ),
            CommandTypes::Inject { path } => execute_inject(ctx, path),
            CommandTypes::Referee(args) => execute_referee(ctx, &args.to_command()),
            CommandTypes::Help { command } => {
                execute_command_help(&resolve_baller_dir(&ctx.config), command.as_deref())
            }
            CommandTypes::Version => execute_command_version(),
            CommandTypes::External(args) => {
                let (name, rest) = args
                    .split_first()
                    .ok_or_else(|| BallError::UnsupportedCommand("<empty>".to_string()))?;
                execute_external(ctx, name, rest)
            }
        }
    }
}

/// True when clap "failed" only because it was asked to print help or version.
fn is_display_request(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::DisplayHelp
            | ErrorKind::DisplayVersion
            | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn parse(args: &[&str]) -> BallerCommand {
        BallerCommand::try_parse_from(args).unwrap()
    }

    #[test]
    fn test_cli_definition_is_valid() {
        BallerCommand::command().debug_assert();
    }

    #[test]
    fn test_global_yes_works_before_and_after_subcommand() {
        let before = parse(&["baller", "-y", "eject", "pkg"]);
        let after = parse(&["baller", "eject", "pkg", "-y"]);
        assert!(before.yes);
        assert!(after.yes);
    }

    #[test]
    fn test_global_flags_collected() {
        let cmd = parse(&[
            "baller",
            "--quiet",
            "--json",
            "--no-hooks",
            "--no-color",
            "--config",
            "/tmp/baller",
            "-v",
            "roster",
        ]);

        let flags = cmd.global_flags();
        assert!(flags.quiet);
        assert!(flags.json);
        assert!(flags.is_quiet());
        assert!(flags.verbose);
        assert!(cmd.no_hooks);
        assert!(cmd.no_color);
        assert_eq!(cmd.config.as_deref(), Some("/tmp/baller"));
    }

    #[test]
    fn test_global_flags_default_off() {
        let flags = parse(&["baller", "roster"]).global_flags();
        assert!(!flags.yes);
        assert!(!flags.quiet);
        assert!(!flags.json);
        assert!(!flags.is_quiet());
        assert!(!flags.verbose);
    }

    #[test]
    fn test_draft_flags() {
        let cmd = parse(&[
            "baller",
            "draft",
            "ripgrep",
            "--version",
            "14.1.0",
            "--source",
            "chocolatey",
            "--no-deps",
            "--dry-run",
            "--force",
        ]);

        match cmd.command {
            CommandTypes::Draft {
                package_name,
                version,
                source,
                no_deps,
                dry_run,
                force,
            } => {
                assert_eq!(package_name, "ripgrep");
                assert_eq!(version.as_deref(), Some("14.1.0"));
                assert_eq!(source, Some(SourceArg::Chocolatey));
                assert!(no_deps && dry_run && force);
            }
            other => panic!("expected Draft, got {:?}", other),
        }
    }

    #[test]
    fn test_draft_rejects_unknown_source() {
        assert!(
            BallerCommand::try_parse_from(["baller", "draft", "x", "--source", "npm"]).is_err()
        );
    }

    #[test]
    fn test_source_arg_maps_to_registry_source() {
        assert_eq!(
            SourceArg::Github.to_registry_source(),
            RegistrySource::GitHub
        );
        assert_eq!(
            SourceArg::Baller.to_registry_source(),
            RegistrySource::BallerRegistry
        );
        assert_eq!(
            SourceArg::Chocolatey.to_registry_source(),
            RegistrySource::Chocolatey
        );
        assert_eq!(
            SourceArg::System.to_registry_source(),
            RegistrySource::System
        );
    }

    #[test]
    fn test_eject_flags() {
        let cmd = parse(&[
            "baller",
            "eject",
            "pkg",
            "--force",
            "--purge",
            "--no-orphans",
            "--keep-bin",
        ]);

        match cmd.command {
            CommandTypes::Eject {
                force,
                purge,
                no_orphans,
                keep_bin,
                ..
            } => assert!(force && purge && no_orphans && keep_bin),
            other => panic!("expected Eject, got {:?}", other),
        }
    }

    #[test]
    fn test_freeze_direction_conflict() {
        assert!(
            BallerCommand::try_parse_from(["baller", "freeze", "pkg", "--freeze", "--thaw"])
                .is_err()
        );
    }

    #[test]
    fn test_freeze_without_package_name() {
        let cmd = parse(&["baller", "freeze", "--list"]);
        match cmd.command {
            CommandTypes::Freeze {
                package_name, list, ..
            } => {
                assert!(package_name.is_none());
                assert!(list);
            }
            other => panic!("expected Freeze, got {:?}", other),
        }
    }

    #[test]
    fn test_sweep_defaults_and_aliases() {
        let default_sweep = parse(&["baller", "sweep"]);
        match default_sweep.command {
            CommandTypes::Sweep {
                all,
                dry_run,
                threshold,
            } => {
                assert!(!all, "sweep must default to archives-only");
                assert!(!dry_run);
                assert!(threshold.is_none());
            }
            other => panic!("expected Sweep, got {:?}", other),
        }

        let aliased = parse(&["baller", "sweep", "--purge-extracted"]);
        match aliased.command {
            CommandTypes::Sweep { all, .. } => assert!(all),
            other => panic!("expected Sweep, got {:?}", other),
        }
    }

    #[test]
    fn test_sweep_threshold_value() {
        let cmd = parse(&["baller", "sweep", "--threshold", "50MB"]);
        match cmd.command {
            CommandTypes::Sweep { threshold, .. } => {
                assert_eq!(threshold.as_deref(), Some("50MB"))
            }
            other => panic!("expected Sweep, got {:?}", other),
        }
    }

    #[test]
    fn test_update_positionals_and_check_alias() {
        let cmd = parse(&["baller", "update", "fzf", "jq", "--dry-run"]);
        match cmd.command {
            CommandTypes::Update {
                packages,
                check,
                include_frozen,
            } => {
                assert_eq!(packages, vec!["fzf".to_string(), "jq".to_string()]);
                assert!(check);
                assert!(!include_frozen);
            }
            other => panic!("expected Update, got {:?}", other),
        }
    }

    #[test]
    fn test_roster_filters() {
        let cmd = parse(&[
            "baller",
            "roster",
            "--frozen",
            "--source",
            "github",
            "--outdated",
            "--remote",
        ]);
        match cmd.command {
            CommandTypes::Roster {
                frozen,
                source,
                outdated,
                remote,
                ..
            } => {
                assert!(frozen && outdated && remote);
                assert_eq!(source, Some(SourceArg::Github));
            }
            other => panic!("expected Roster, got {:?}", other),
        }
    }

    #[test]
    fn test_substitute_flags() {
        let cmd = parse(&[
            "baller",
            "substitute",
            "old",
            "new",
            "--keep-old",
            "--no-deps",
        ]);
        match cmd.command {
            CommandTypes::Substitute {
                old_package,
                new_package,
                keep_old,
                dry_run,
                no_deps,
            } => {
                assert_eq!(old_package, "old");
                assert_eq!(new_package, "new");
                assert!(keep_old && no_deps);
                assert!(!dry_run);
            }
            other => panic!("expected Substitute, got {:?}", other),
        }
    }

    #[test]
    fn test_build_flags() {
        let cmd = parse(&[
            "baller",
            "build",
            "./pkg",
            "--dry-run",
            "--no-deps",
            "--install-dir",
            "/opt/bin",
            "--force",
            "--source",
            "github",
        ]);
        match cmd.command {
            CommandTypes::Build {
                path,
                dry_run,
                no_deps,
                install_dir,
                force,
                source,
            } => {
                assert_eq!(path, "./pkg");
                assert!(dry_run && no_deps && force);
                assert_eq!(install_dir.as_deref(), Some("/opt/bin"));
                assert_eq!(source, Some(SourceArg::Github));
            }
            other => panic!("expected Build, got {:?}", other),
        }
    }

    #[test]
    fn test_version_subcommand() {
        let cmd = parse(&["baller", "version"]);
        match cmd.command {
            CommandTypes::Version => {}
            other => panic!("expected Version, got {:?}", other),
        }
    }

    fn referee(args: &[&str]) -> RefereeCommand {
        match parse(args).command {
            CommandTypes::Referee(referee) => referee.to_command(),
            other => panic!("expected Referee, got {:?}", other),
        }
    }

    fn referee_rejects(args: &[&str]) {
        assert!(
            BallerCommand::try_parse_from(args).is_err(),
            "{:?} should be rejected",
            args
        );
    }

    #[test]
    fn test_bare_referee_is_an_audit_of_the_roster() {
        match referee(&["baller", "referee"]) {
            RefereeCommand::Audit(opts) => {
                assert!(opts.package_names.is_empty());
                assert!(!opts.refresh && !opts.no_scan);
                assert!(opts.fail_on.is_none() && opts.format.is_none() && opts.out.is_none());
            }
            _ => panic!("expected an audit"),
        }
    }

    #[test]
    fn test_referee_positional_form_still_audits_one_package() {
        match referee(&["baller", "referee", "fd", "--refresh", "--no-scan"]) {
            RefereeCommand::Audit(opts) => {
                assert_eq!(opts.package_names, vec!["fd".to_string()]);
                assert!(opts.refresh && opts.no_scan);
            }
            _ => panic!("expected an audit"),
        }
    }

    #[test]
    fn test_referee_audit_subcommand_takes_many_packages_and_export_flags() {
        match referee(&[
            "baller",
            "referee",
            "audit",
            "fd",
            "ripgrep",
            "--fail-on",
            "warn",
            "--format",
            "sarif",
            "--out",
            "report.sarif",
        ]) {
            RefereeCommand::Audit(opts) => {
                assert_eq!(
                    opts.package_names,
                    vec!["fd".to_string(), "ripgrep".to_string()]
                );
                assert_eq!(opts.fail_on, Some(FailOn::Warn));
                assert_eq!(opts.format, Some(AuditFormat::Sarif));
                assert_eq!(opts.out.as_deref(), Some("report.sarif"));
            }
            _ => panic!("expected an audit"),
        }
    }

    #[test]
    fn test_referee_check_and_scan_subcommands() {
        match referee(&[
            "baller",
            "referee",
            "check",
            "fd",
            "--refresh",
            "--fail-on",
            "block",
        ]) {
            RefereeCommand::Check {
                package_names,
                refresh,
                fail_on,
            } => {
                assert_eq!(package_names, vec!["fd".to_string()]);
                assert!(refresh);
                assert_eq!(fail_on, Some(FailOn::Block));
            }
            _ => panic!("expected check"),
        }
        match referee(&["baller", "referee", "scan"]) {
            RefereeCommand::Scan { package_names } => assert!(package_names.is_empty()),
            _ => panic!("expected scan"),
        }
    }

    #[test]
    fn test_referee_cache_actions() {
        assert!(matches!(
            referee(&["baller", "referee", "cache"]),
            RefereeCommand::Cache(CacheAction::Status)
        ));
        assert!(matches!(
            referee(&["baller", "referee", "cache", "--status"]),
            RefereeCommand::Cache(CacheAction::Status)
        ));
        assert!(matches!(
            referee(&["baller", "referee", "cache", "--clear"]),
            RefereeCommand::Cache(CacheAction::Clear)
        ));
        assert!(matches!(
            referee(&["baller", "referee", "cache", "--prune", "30"]),
            RefereeCommand::Cache(CacheAction::Prune(30))
        ));
    }

    #[test]
    fn test_referee_cache_actions_are_mutually_exclusive() {
        referee_rejects(&["baller", "referee", "cache", "--status", "--clear"]);
        referee_rejects(&["baller", "referee", "cache", "--status", "--prune", "3"]);
        referee_rejects(&["baller", "referee", "cache", "--clear", "--prune", "3"]);
        referee_rejects(&["baller", "referee", "cache", "--prune", "-1"]);
        referee_rejects(&["baller", "referee", "cache", "--prune", "soon"]);
    }

    #[test]
    fn test_referee_config_and_sbom() {
        assert!(matches!(
            referee(&["baller", "referee", "config", "--json"]),
            RefereeCommand::Config
        ));
        match referee(&["baller", "referee", "sbom"]) {
            RefereeCommand::Sbom { out, format } => {
                assert!(out.is_none());
                assert_eq!(format, SbomFormat::CyclonedxJson);
            }
            _ => panic!("expected sbom"),
        }
        match referee(&[
            "baller",
            "referee",
            "sbom",
            "--out",
            "bom.json",
            "--format",
            "cyclonedx-json",
        ]) {
            RefereeCommand::Sbom { out, .. } => assert_eq!(out.as_deref(), Some("bom.json")),
            _ => panic!("expected sbom"),
        }
    }

    #[test]
    fn test_referee_rejects_invalid_values_and_mixed_forms() {
        referee_rejects(&["baller", "referee", "audit", "--fail-on", "pass"]);
        referee_rejects(&["baller", "referee", "check", "--fail-on", "critical"]);
        referee_rejects(&["baller", "referee", "audit", "--format", "xml"]);
        referee_rejects(&["baller", "referee", "sbom", "--format", "spdx"]);
        // --out needs a format to write.
        referee_rejects(&["baller", "referee", "audit", "--out", "report.json"]);
        // Export and fail-on flags belong to the subcommands, not the bare form.
        referee_rejects(&["baller", "referee", "--fail-on", "block"]);
        // A package and a subcommand cannot be mixed.
        referee_rejects(&["baller", "referee", "fd", "audit"]);
        // Once a bare-form flag is given, a subcommand name is just a package.
        match referee(&["baller", "referee", "--refresh", "check"]) {
            RefereeCommand::Audit(opts) => {
                assert_eq!(opts.package_names, vec!["check".to_string()])
            }
            _ => panic!("expected an audit of a package named 'check'"),
        }
        // scan takes no advisory flags.
        referee_rejects(&["baller", "referee", "scan", "--refresh"]);
    }
}
