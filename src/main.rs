mod utils;
mod error;
mod cli;
mod commands;
mod config;

use std::{env::{self, home_dir}, fs::create_dir_all, path::PathBuf, process::exit};

use crate::{cli::parse::BallerCommand, config::config::BallerConfig, error::error::BallError};

// crate version to be printed when 'baller' or 'baller version' is run
pub const CRATE_VERSION: &str = "v0.1";

fn main() {
    if let Err(e) = entry() {
        eprintln!("\n[Error]: {}", e);
        exit(1);
    }
}

fn entry() -> Result<(), BallError> {
    // return an error if the compile target is not linux or windows
    if !cfg!(target_os = "linux") && !cfg!(target_os = "windows") {
        return Err(BallError::UnsupportedOs(env::consts::OS.to_string()));
    }

    let baller_dir: String = create_baller_dir()?;
    let baller_config: BallerConfig = BallerConfig::parse_config(&baller_dir)?;

    println!("{:?}", baller_config);

    // parse command
    let command: BallerCommand = BallerCommand::parse_command()?;

    command.execute()?;

    // the following print is only for debug and may be commented and uncommented when needed
    // println!("command: {:?}", command);

    return Ok(());
}

// create the default baller directory if it doesn't already exist
fn create_baller_dir() -> Result<String, BallError> {
    let home_path: PathBuf = home_dir().unwrap();
    let mut baller_dir: String = home_path.display().to_string();
    
    if cfg!(target_os = "linux") {
        baller_dir.push_str("/.baller");
    } else if cfg!(target_os = "windows") {
        baller_dir.push_str("/AppData/Local/baller");
    }

    create_dir_all(&baller_dir).map_err(|e| BallError::FileIoErr(e))?;

    return Ok(baller_dir);
}