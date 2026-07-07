use crate::{error::error::BallError, CRATE_VERSION};

#[allow(dead_code)]
pub fn execute_command_version() -> Result<(), BallError> {
    println!("Baller {}", CRATE_VERSION);

    Ok(())
}
