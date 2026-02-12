use crate::error::error::BallError;

#[derive(Debug)]
pub struct UpdateParameters {
    // parameters for the update command
}

pub fn parse_update_parameters(_args: &Vec<String>) -> Result<UpdateParameters, BallError> {
    let parameters: UpdateParameters = UpdateParameters {  };

    return Ok(parameters);
}

pub fn execute_command_update(_update_parameters: &UpdateParameters) -> Result<(), BallError> {
    return Ok(());
}