use crate::error::error::BallError;

#[allow(dead_code)]
pub fn execute_command_help() -> Result<(), BallError> {
    println!("Available commands:");
    println!("draft <name>: installs a package. Flags: --version <v>, --source <s>, --no-deps, --dry-run, -f/--force");
    println!(
        "eject <name>: uninstalls a package. Flags: -f/--force, --purge, --no-orphans, --keep-bin"
    );
    println!("roster [name]: lists installed packages or searches for one. Flags: --frozen, --source <s>, --outdated, --remote");
    println!("freeze [name]: pins a package so it cannot be updated or ejected. Flags: --freeze, --thaw, --all, --list");
    println!("substitute <old> <new>: swaps a package for a compatible one (e.g replacing 'firefox' with 'edge'). Flags: --keep-old, --dry-run, --no-deps");
    println!("sweep: removes cached package archives. Flags: --all, --dry-run, --threshold <size>");
    println!("update [packages...]: updates active packages. Flags: --check, --include-frozen");
    println!("build <path>: builds a package from a local manifest. Flags: --dry-run, --no-deps, --install-dir <dir>, -f/--force, --source <s>");
    println!("help: outputs available commands");
    println!();
    println!("Global flags (work on every command):");
    println!("  -y/--yes        skip confirmation prompts");
    println!("  -q/--quiet      suppress progress bars and step-by-step output");
    println!("  -v/--verbose    increase output detail (repeatable)");
    println!("  --json          emit machine-readable JSON");
    println!("  --no-hooks      skip every install/eject/update hook");
    println!("  --no-color      disable colored output");
    println!("  --config <dir>  use an alternate baller directory");
    println!();
    println!(
        "Note: sweep clears downloaded archives only; pass --all to also drop extracted packages."
    );

    Ok(())
}
