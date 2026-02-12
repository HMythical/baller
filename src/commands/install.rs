use crate::error::error::BallError;

#[derive(Debug)]
pub struct InstallParameters {
    // parameters for the install command
}

pub fn parse_install_parameters(_args: &Vec<String>) -> Result<InstallParameters, BallError> {
    let parameters: InstallParameters = InstallParameters {  };

    return Ok(parameters);
}

pub fn execute_command_install(_install_parameters: &InstallParameters) -> Result<(), BallError> {
    return Ok(());
}