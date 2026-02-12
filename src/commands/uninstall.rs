use crate::error::error::BallError;

#[derive(Debug)]
pub struct UninstallParameters {
    // parameters for the uninstall command
}

pub fn parse_uninstall_parameters(_args: &Vec<String>) -> Result<UninstallParameters, BallError> {
    let parameters: UninstallParameters = UninstallParameters {  };

    return Ok(parameters);
}

pub fn execute_command_uninstall(_uninstall_parameters: &UninstallParameters) -> Result<(), BallError> {
    return Ok(());
}