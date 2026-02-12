use crate::{CRATE_VERSION, error::error::BallError};

pub fn execute_command_version() -> Result<(), BallError> {
    println!("Baller {}", CRATE_VERSION);

    return Ok(());
}