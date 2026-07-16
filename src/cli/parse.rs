use crate::commands::{
    build::execute_build, draft::execute_draft, eject::execute_eject, freeze::execute_freeze,
    roster::execute_roster, substitute::execute_substitute, sweep::execute_sweep,
    update::execute_update,
};
use crate::context::AppContext;
use crate::error::error::BallError;
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "baller")]
#[command(about = "B.A.L.L.E.R - The Binary Allocation & Library Launch Environment in Rust", long_about = None)]
#[command(version)]
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
}

impl BallerCommand {
    pub fn parse_command() -> Result<Self, BallError> {
        let cmd = BallerCommand::try_parse()
            .map_err(|e| BallError::InvalidConfig(format!("CLI Error: {}", e)))?;
        Ok(cmd)
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
        }
    }
}
