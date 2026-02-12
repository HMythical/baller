use crate::error::error::BallError;

pub fn execute_command_help() -> Result<(), BallError> {
    println!("list of commands available:\n\tclean: expl\n\tversion: prints the current version of Baller\n\thelp: outputs available commands\n\tlist: exlp\n\tinstall: installs a package\n\tuninstall: uninstalls a package\n\tupdate: updates a package\nnote: if no commands are passed in, it will default to 'baller version'");

    return Ok(());
}