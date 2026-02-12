use crate::error::error::BallError;

#[derive(Debug)]
pub struct ListParameters {
    list_dependencies: bool
}

pub fn parse_list_parameters(args: &Vec<String>) -> Result<ListParameters, BallError> {
    let mut list_dependencies: bool = false;

    // check for the -d parameter and return an error if an unknown parameter is found
    if args.len() > 2 {
        match args[2].as_str() {
            "-d" => list_dependencies = true,
            _ => { return Err(BallError::UnknownParameter(args[2].clone())); }
        };
    }

    let parameters: ListParameters = ListParameters {
        list_dependencies
    };

    return Ok(parameters);
}

pub fn execute_command_list(_list_parameters: &ListParameters) -> Result<(), BallError> {
    println!("{:?}", _list_parameters.list_dependencies);
    return Ok(());
}