mod utils;
mod error;
mod cli;
mod commands;
mod config;

use std::{env, process::exit};

use crate::{cli::parse::BallerCommand, error::error::BallError};

// crate version to be printed when 'baller' or 'baller version' is run
pub const CRATE_VERSION: &str = "v0.1";

fn main() {
    if let Err(e) = entry() {
        eprintln!("[Error]: {}", e);
        exit(1);
    }
}

fn entry() -> Result<(), BallError> {
    // return an error if the compile target is not linux or windows
    if !cfg!(target_os = "linux") && !cfg!(target_os = "windows") {
        return Err(BallError::UnsupportedOs(env::consts::OS.to_string()));
    }

    // parse command
    let command: BallerCommand = BallerCommand::parse_command()?;

    command.execute()?;

    // the following print is only for debug and may be commented and uncommented when needed
    // println!("command: {:?}", command);

    return Ok(());
}