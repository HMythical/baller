use crate::error::error::BallError;

#[allow(dead_code)]
pub fn execute_command_help() -> Result<(), BallError> {
    // TODO: replace expl with actual explanations for the commands and print per command parameter options
    println!("Available commands:");
    println!("sweep: Removes cached package archives, orphaned dependencies, broken symlinks, stale lock files.");
    println!("version: prints the current version of Baller");
    println!("help: outputs available commands");
    println!("roster: lists all installed packages");
    println!("draft: installs a package. Flags: ");
    println!("eject: uninstalls a package");
    println!("update: updates a package");
    println!("build: builds a specific package from a manifest");
    println!("freeze: freezes a package (prevents it from being updated by baller)");
    println!("substitute: swaps a package with a different, compatible package. (e.g replacing 'firefox' with 'edge')");
    println!("Note: if no commands are passed in, it will default to 'baller version'");

    Ok(())
}
