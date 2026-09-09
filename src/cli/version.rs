use crate::error::error::BallError;

pub fn execute_command_version() -> Result<(), BallError> {
    println!("Baller {}", env!("CARGO_PKG_VERSION"));

    Ok(())
}
