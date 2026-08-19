use crate::cli::help::execute_command_help;
use crate::commands::{
    build::execute_build, draft::execute_draft, eject::execute_eject, external::execute_external,
    freeze::execute_freeze, inject::execute_inject, roster::execute_roster,
    substitute::execute_substitute, sweep::execute_sweep, update::execute_update,
};
use crate::context::AppContext;
use crate::core::injected::resolve_baller_dir;
use crate::error::error::BallError;
use clap::error::ErrorKind;
use clap::{Parser, Subcommand};

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
}

#[derive(Subcommand, Debug)]
pub enum CommandTypes {
    /// Drafts (Installs) a new player onto your team
    Draft { package_name: String },
    /// Ejects (Uninstalls) a player from your team
    #[command(arg_required_else_help = false)]
    Eject {
        package_name: String,
        /// Skip confirmation prompt
        #[arg(short, long)]
        yes: bool,
    },
    /// Freezes (Pins) a player so they cannot be substituted or updated
    Freeze { package_name: String },
    /// Rosters (Lists) active players on your team or searches for one
    Roster { package_name: Option<String> },
    /// Substitutes (Swaps) a current player for a new one cleanly
    Substitute {
        old_package: String,
        new_package: String,
    },
    /// Sweeps (Cleans) the arena of leftover caching debris
    #[command(arg_required_else_help = false)]
    Sweep {
        /// Skip confirmation prompt
        #[arg(short, long)]
        yes: bool,
    },
    /// Updates all active packages on the team
    Update,
    /// Builds a package natively from a local manifest
    Build { path: String },
    /// Injects a custom command described by a .ball file
    Inject { path: String },
    /// Prints every available command, or details for one of them
    Help {
        /// Built-in or injected command to describe
        command: Option<String>,
    },
    /// Any unknown subcommand: dispatched to an injected command, if one matches
    #[command(external_subcommand)]
    External(Vec<String>),
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

    pub fn execute(&self, ctx: &AppContext) -> Result<(), BallError> {
        match &self.command {
            CommandTypes::Draft { package_name } => execute_draft(ctx, package_name),
            CommandTypes::Eject { package_name, yes } => execute_eject(ctx, package_name, *yes),
            CommandTypes::Freeze { package_name } => execute_freeze(ctx, package_name),
            CommandTypes::Roster { package_name } => execute_roster(ctx, package_name),
            CommandTypes::Substitute {
                old_package,
                new_package,
            } => execute_substitute(ctx, old_package, new_package),
            CommandTypes::Sweep { yes } => execute_sweep(ctx, *yes),
            CommandTypes::Update => execute_update(ctx),
            CommandTypes::Build { path } => execute_build(ctx, path),
            CommandTypes::Inject { path } => execute_inject(ctx, path),
            CommandTypes::Help { command } => {
                execute_command_help(&resolve_baller_dir(&ctx.config), command.as_deref())
            }
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
