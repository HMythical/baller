use std::env;

use crate::{cli::{help::execute_command_help, version::execute_command_version}, commands::{clean::execute_command_clean, install::{InstallParameters, execute_command_install, parse_install_parameters}, list::execute_command_list, uninstall::{UninstallParameters, execute_command_uninstall, parse_uninstall_parameters}, update::{UpdateParameters, execute_command_update, parse_update_parameters}}, error::error::BallError};

// commands that take in parameters require a parameter struct
#[derive(Debug)]
pub enum CommandTypes {
    Clean,
    Install(InstallParameters),
    List,
    Uninstall(UninstallParameters),
    Update(UpdateParameters),
    Help,
    Version
}

#[derive(Debug)]
pub struct BallerCommand {
    ty: CommandTypes,
}

impl BallerCommand {
    pub fn parse_command() -> Result<Self, BallError> {
        let mut args: Vec<String> = env::args().collect();

        // if no command is specified treat it as 'baller version'
        if args.len() < 2 {
            args.push("version".to_string());
        }
        
        /*
         * note: arg[0] is the program itself (baller binary)
         * commands that take parameters need a parse parameters function (declared in commands/<command name>.rs)
         */
        let command_ty: CommandTypes = match args[1].as_str() {
            "clean" => CommandTypes::Clean,
            "install" => CommandTypes::Install(parse_install_parameters(&args)?),
            "list" => CommandTypes::List,
            "uninstall" => CommandTypes::Uninstall(parse_uninstall_parameters(&args)?),
            "update" => CommandTypes::Update(parse_update_parameters(&args)?),
            "help" => CommandTypes::Help,
            "version" => CommandTypes::Version,
            _ => return Err(BallError::UnsupportedCommand(args[1].clone()))
        };

        let final_command: BallerCommand = BallerCommand {
            ty: command_ty
        };

        return Ok(final_command);
    }

    pub fn execute(&self) -> Result<(), BallError> {
        /*
         * execute the function corresponding to the command
         * the functions are in commands/<command name>.rs for the exception of help and version
         */
        return match &self.ty {
            CommandTypes::Clean => execute_command_clean(),
            CommandTypes::Install(params) => execute_command_install(params),
            CommandTypes::List => execute_command_list(),
            CommandTypes::Uninstall(params) => execute_command_uninstall(params),
            CommandTypes::Update(params) => execute_command_update(params),
            CommandTypes::Help => execute_command_help(),
            CommandTypes::Version => execute_command_version(),
        };
    }
}