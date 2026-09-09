use colored::Colorize;

use crate::core::injected::{load_injected, InjectedCommand};
use crate::error::error::BallError;

const TAGLINE: &str = "B.A.L.L.E.R - The Binary Allocation & Library Launch Environment in Rust";

/// One entry of baller's help table.
///
/// Kept hand-written rather than derived from clap so `baller help` can list
/// built-in and injected commands side by side in the same layout.
struct BuiltinCommand {
    name: &'static str,
    summary: &'static str,
    usage: &'static str,
    /// Argument and flag lines, already formatted as `name  explanation`.
    details: &'static [&'static str],
    /// Free-form paragraphs shown under the arguments.
    notes: &'static [&'static str],
}

const BUILTIN_COMMANDS: [BuiltinCommand; 11] = [
    BuiltinCommand {
        name: "draft",
        summary: "Drafts (Installs) a new player onto your team",
        usage: "baller draft <PACKAGE_NAME>",
        details: &[
            "<PACKAGE_NAME>  Package to install",
            "--version <V>   Pin an exact version (GitHub and Chocolatey sources only)",
            "--source <SRC>  Resolve from one registry instead of the configured chain",
            "--no-deps       Install the package alone, ignoring its dependencies",
            "--dry-run       Show what would be installed without touching anything",
            "-f, --force     Reinstall even when the package is already on the roster",
        ],
        notes: &[],
    },
    BuiltinCommand {
        name: "eject",
        summary: "Ejects (Uninstalls) a player from your team",
        usage: "baller eject <PACKAGE_NAME>",
        details: &[
            "<PACKAGE_NAME>  Package to uninstall",
            "-f, --force     Eject even when the package is frozen",
            "--purge         Also delete the cached download archive",
            "--no-orphans    Leave orphaned dependencies installed",
            "--keep-bin      Drop the roster entry but leave the linked binary in place",
        ],
        notes: &["Frozen packages must be unfrozen before they can be ejected."],
    },
    BuiltinCommand {
        name: "freeze",
        summary: "Freezes (Pins) a player so they cannot be substituted or updated",
        usage: "baller freeze <PACKAGE_NAME>",
        details: &[
            "<PACKAGE_NAME>  Package to pin at its current version",
            "--freeze        Freeze explicitly instead of toggling",
            "--thaw          Thaw explicitly instead of toggling",
            "--all           Apply to every installed package",
            "--list          List the frozen packages",
        ],
        notes: &[],
    },
    BuiltinCommand {
        name: "roster",
        summary: "Rosters (Lists) active players on your team or searches for one",
        usage: "baller roster [PACKAGE_NAME]",
        details: &[
            "[PACKAGE_NAME]  Package to look up; omit to list everything installed",
            "--frozen        Only show frozen packages",
            "--source <SRC>  Only show packages installed from this source",
            "--outdated      Check registries for newer versions without updating",
            "--remote        Search registries instead of the local roster",
        ],
        notes: &[],
    },
    BuiltinCommand {
        name: "substitute",
        summary: "Substitutes (Swaps) a current player for a new one cleanly",
        usage: "baller substitute <OLD_PACKAGE> <NEW_PACKAGE>",
        details: &[
            "<OLD_PACKAGE>  Package to remove",
            "<NEW_PACKAGE>  Package to install in its place",
            "--keep-old     Install the new package but leave the old one installed",
            "--dry-run      Show what would change without touching anything",
            "--no-deps      Install the new package alone, ignoring its dependencies",
        ],
        notes: &[],
    },
    BuiltinCommand {
        name: "sweep",
        summary: "Sweeps (Cleans) the arena of leftover caching debris",
        usage: "baller sweep",
        details: &[
            "--all                 Also delete extracted packages, not just downloaded archives",
            "--dry-run             Report what would be swept without deleting",
            "--threshold <SIZE>    Only sweep when the cache is larger than this (e.g. 50MB)",
        ],
        notes: &["Removes downloaded archives only; pass --all to also drop extracted packages."],
    },
    BuiltinCommand {
        name: "update",
        summary: "Updates all active packages on the team",
        usage: "baller update [PACKAGES...]",
        details: &[
            "[PACKAGES...]  Update only these packages (default: everything)",
            "--check        Report stale packages without updating them",
            "--include-frozen  Update frozen packages too",
        ],
        notes: &["Frozen packages are skipped by default."],
    },
    BuiltinCommand {
        name: "build",
        summary: "Builds a package natively from a local manifest",
        usage: "baller build <PATH>",
        details: &[
            "<PATH>           Manifest (.toml or .json) to build from, or a directory",
            "                 containing a Cargo project (Rust sources are compiled",
            "                 with 'cargo build --release' and installed)",
            "--dry-run        Parse and validate the manifest without installing",
            "--no-deps        Ignore the manifest's declared dependencies",
            "--install-dir    Link the binary into this directory instead of the default",
            "-f, --force      Build over an existing installation of the same package",
            "--source <SRC>   Override the manifest's source before resolving",
        ],
        notes: &[
            "A Cargo project's dependencies are resolved by cargo itself, so",
            "--source and --no-deps do not apply to one.",
        ],
    },
    BuiltinCommand {
        name: "inject",
        summary: "Injects a custom command described by a .ball file",
        usage: "baller inject <PATH>",
        details: &["<PATH>  .ball file describing the command to add"],
        notes: &[
            "Injecting takes three confirmations: the named binary then runs as",
            "'baller <command-name>' with your privileges.",
            "Injected commands are stored in injected_commands.json under baller's",
            "config directory, and are listed at the bottom of 'baller help'.",
        ],
    },
    BuiltinCommand {
        name: "help",
        summary: "Prints this message, or details for one command",
        usage: "baller help [COMMAND]",
        details: &["[COMMAND]  Built-in or injected command to describe"],
        notes: &[],
    },
    BuiltinCommand {
        name: "version",
        summary: "Prints the current baller version",
        usage: "baller version",
        details: &[],
        notes: &[],
    },
];

const GLOBAL_OPTIONS: [&str; 9] = [
    "-y, --yes          Skip confirmation prompts",
    "-q, --quiet        Suppress progress bars and step-by-step output",
    "-v, --verbose      Increase output detail",
    "--json             Emit machine-readable JSON instead of formatted text",
    "--no-hooks         Skip every pre/post install, eject and update hook",
    "--no-color         Disable colored output",
    "--config <DIR>     Use an alternate baller directory",
    "-h, --help         Print a short usage summary",
    "-V, --version      Print baller's version",
];

/// Prints the command overview, or the details of a single command.
///
/// `topic` resolves against built-ins first, then injected commands, so a
/// built-in can never be shadowed in help output.
pub fn execute_command_help(baller_dir: &str, topic: Option<&str>) -> Result<(), BallError> {
    let injected = load_injected(baller_dir);

    let Some(name) = topic else {
        println!("{}", render_overview(&injected));
        return Ok(());
    };

    if let Some(builtin) = find_builtin(name) {
        println!("{}", render_builtin_detail(builtin));
        return Ok(());
    }

    match injected.iter().find(|c| c.command_name == name) {
        Some(command) => {
            println!("{}", render_injected_detail(command));
            Ok(())
        }
        None => Err(BallError::UnsupportedCommand(name.to_string())),
    }
}

fn find_builtin(name: &str) -> Option<&'static BuiltinCommand> {
    BUILTIN_COMMANDS.iter().find(|c| c.name == name)
}

/// Every command baller can run, built-in first and injected last.
fn render_overview(injected: &[InjectedCommand]) -> String {
    let width = column_width(injected);
    let mut out = String::new();

    out.push_str(TAGLINE);
    out.push_str("\n\n");
    out.push_str(&format!(
        "{} baller <COMMAND> [ARGS]...\n\n",
        "Usage:".bold()
    ));

    out.push_str(&format!("{}\n", "Commands:".bold()));
    for command in BUILTIN_COMMANDS.iter() {
        out.push_str(&entry_line(command.name, command.summary, width));
    }

    if injected.is_empty() {
        out.push_str(&format!(
            "\nNo injected commands yet. Add one with '{}'.\n",
            "baller inject <PATH>".cyan()
        ));
    } else {
        out.push_str(&format!("\n{}\n", "Injected commands:".bold()));
        for command in injected {
            out.push_str(&entry_line(
                &command.command_name,
                &describe(command),
                width,
            ));
        }
    }

    out.push_str(&format!("\n{}\n", "Global options:".bold()));
    for option in GLOBAL_OPTIONS {
        out.push_str(&format!("  {}\n", option));
    }

    out.push_str(&format!(
        "\nRun '{}' for details on a specific command.",
        "baller help <COMMAND>".cyan()
    ));

    out
}

fn render_builtin_detail(command: &BuiltinCommand) -> String {
    let mut out = String::new();

    out.push_str(&format!(
        "{} - {}\n\n",
        command.name.cyan().bold(),
        command.summary
    ));
    out.push_str(&format!("{} {}\n", "Usage:".bold(), command.usage));

    if !command.details.is_empty() {
        out.push('\n');
        for line in command.details {
            out.push_str(&format!("  {}\n", line));
        }
    }

    if !command.notes.is_empty() {
        out.push('\n');
        for line in command.notes {
            out.push_str(&format!("{}\n", line));
        }
    }

    out.trim_end().to_string()
}

fn render_injected_detail(command: &InjectedCommand) -> String {
    let mut out = String::new();

    out.push_str(&format!(
        "{} - {} {}\n\n",
        command.command_name.cyan().bold(),
        describe(command),
        "(injected)".dimmed()
    ));
    out.push_str(&format!(
        "{} baller {} [ARGS]...\n\n",
        "Usage:".bold(),
        command.command_name
    ));

    out.push_str(&format!("  Version:       {}\n", command.version));

    if !command.author.is_empty() {
        out.push_str(&format!("  Author:        {}\n", command.author));
    }

    if !command.flags.is_empty() {
        out.push_str(&format!("  Flags:         {}\n", command.flags.join(", ")));
    }

    if !command.depends.is_empty() {
        out.push_str(&format!(
            "  Requires:      {}\n",
            command.depends.join(", ")
        ));
    }

    out.push_str(&format!(
        "  Requires root: {}\n",
        if command.require_root { "yes" } else { "no" }
    ));
    out.push_str(&format!("  Binary:        {}\n", command.path.display()));

    out.push_str("\nArguments and flags are passed straight through to the binary.");

    out
}

/// Pads on the plain name so colour escapes never skew the columns.
fn entry_line(name: &str, summary: &str, width: usize) -> String {
    let padding = " ".repeat(width.saturating_sub(name.chars().count()));
    format!("  {}{}  {}\n", name.cyan(), padding, summary)
}

fn column_width(injected: &[InjectedCommand]) -> usize {
    BUILTIN_COMMANDS
        .iter()
        .map(|c| c.name.chars().count())
        .chain(injected.iter().map(|c| c.command_name.chars().count()))
        .max()
        .unwrap_or(0)
}

fn describe(command: &InjectedCommand) -> String {
    if command.description.is_empty() {
        "no description provided".to_string()
    } else {
        command.description.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::parse::BallerCommand;
    use crate::core::injected::save_injected;
    use clap::CommandFactory;
    use std::path::PathBuf;

    fn test_dir(tag: &str) -> String {
        let dir = std::env::temp_dir().join("baller_help_tests").join(format!(
            "{}_{}",
            std::process::id(),
            tag
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.to_string_lossy().to_string()
    }

    fn sample() -> InjectedCommand {
        InjectedCommand {
            command_name: "my-tool".to_string(),
            description: "A helpful tool that does X".to_string(),
            version: "1.0.0".to_string(),
            flags: vec!["-y".to_string(), "--yes".to_string()],
            author: "HMythical".to_string(),
            require_root: true,
            depends: vec!["python3".to_string()],
            path: PathBuf::from("/usr/local/bin/my-tool"),
        }
    }

    #[test]
    fn test_overview_lists_every_builtin() {
        let overview = render_overview(&[]);
        for command in BUILTIN_COMMANDS.iter() {
            assert!(
                overview.contains(command.name),
                "overview is missing '{}'",
                command.name
            );
            assert!(overview.contains(command.summary));
        }
        assert!(overview.contains("Usage:"));
        assert!(overview.contains("Global options:"));
        assert!(overview.contains("--version"));
    }

    #[test]
    fn test_overview_without_injected_commands_hints_at_inject() {
        let overview = render_overview(&[]);
        assert!(overview.contains("No injected commands yet"));
        assert!(!overview.contains("Injected commands:"));
    }

    #[test]
    fn test_overview_lists_injected_commands() {
        let overview = render_overview(&[sample()]);
        assert!(overview.contains("Injected commands:"));
        assert!(overview.contains("my-tool"));
        assert!(overview.contains("A helpful tool that does X"));
    }

    #[test]
    fn test_overview_falls_back_for_missing_description() {
        let mut command = sample();
        command.description = String::new();
        let overview = render_overview(&[command]);
        assert!(overview.contains("no description provided"));
    }

    #[test]
    fn test_overview_columns_align_with_long_injected_name() {
        let mut command = sample();
        command.command_name = "a-very-long-injected-name".to_string();
        let overview = render_overview(&[command.clone()]);

        let builtin_line = overview
            .lines()
            .find(|l| l.trim_start().starts_with("draft"))
            .unwrap();
        let injected_line = overview
            .lines()
            .find(|l| l.trim_start().starts_with(&command.command_name))
            .unwrap();

        assert_eq!(
            builtin_line.find("Drafts").unwrap(),
            injected_line.find("A helpful").unwrap()
        );
    }

    #[test]
    fn test_builtin_detail_shows_usage_and_arguments() {
        let detail = render_builtin_detail(find_builtin("eject").unwrap());
        assert!(detail.contains("eject"));
        assert!(detail.contains("baller eject <PACKAGE_NAME>"));
        assert!(detail.contains("-f, --force"));
        assert!(detail.contains("Frozen packages"));
    }

    #[test]
    fn test_builtin_detail_without_arguments() {
        let detail = render_builtin_detail(find_builtin("update").unwrap());
        assert!(detail.contains("baller update [PACKAGES...]"));
        assert!(detail.contains("Frozen packages are skipped by default."));
    }

    #[test]
    fn test_injected_detail_shows_every_manifest_field() {
        let detail = render_injected_detail(&sample());
        assert!(detail.contains("my-tool"));
        assert!(detail.contains("A helpful tool that does X"));
        assert!(detail.contains("1.0.0"));
        assert!(detail.contains("HMythical"));
        assert!(detail.contains("-y, --yes"));
        assert!(detail.contains("python3"));
        assert!(detail.contains("Requires root: yes"));
        assert!(detail.contains("/usr/local/bin/my-tool"));
    }

    #[test]
    fn test_injected_detail_omits_empty_fields() {
        let mut command = sample();
        command.author = String::new();
        command.flags = vec![];
        command.depends = vec![];
        command.require_root = false;

        let detail = render_injected_detail(&command);
        assert!(!detail.contains("Author:"));
        assert!(!detail.contains("Flags:"));
        assert!(!detail.contains("Requires:  "));
        assert!(detail.contains("Requires root: no"));
    }

    #[test]
    fn test_help_overview_runs() {
        let dir = test_dir("overview");
        assert!(execute_command_help(&dir, None).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_help_topic_resolves_builtin() {
        let dir = test_dir("builtin_topic");
        for command in BUILTIN_COMMANDS.iter() {
            assert!(
                execute_command_help(&dir, Some(command.name)).is_ok(),
                "help failed for '{}'",
                command.name
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_help_topic_resolves_injected_command() {
        let dir = test_dir("injected_topic");
        save_injected(&dir, &[sample()]).unwrap();
        assert!(execute_command_help(&dir, Some("my-tool")).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_help_topic_unknown_is_an_error() {
        let dir = test_dir("unknown_topic");
        let err = execute_command_help(&dir, Some("not-a-command")).unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("not-a-command"));
        assert!(msg.contains("does not exist"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_builtin_wins_over_injected_command_of_the_same_name() {
        let dir = test_dir("shadowing");
        let mut command = sample();
        command.command_name = "draft".to_string();
        save_injected(&dir, &[command]).unwrap();

        // Injection rejects built-in names, but a hand-edited store must not
        // be able to hide the real command's help either.
        assert!(execute_command_help(&dir, Some("draft")).is_ok());
        let detail = render_builtin_detail(find_builtin("draft").unwrap());
        assert!(detail.contains("<PACKAGE_NAME>"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_every_clap_subcommand_is_documented() {
        let command = BallerCommand::command();
        for subcommand in command.get_subcommands() {
            let name = subcommand.get_name();
            assert!(
                find_builtin(name).is_some(),
                "'{}' is a subcommand but has no entry in BUILTIN_COMMANDS",
                name
            );
        }
    }

    #[test]
    fn test_every_documented_command_is_reserved_from_injection() {
        use crate::commands::inject::is_reserved;
        for command in BUILTIN_COMMANDS.iter() {
            assert!(
                is_reserved(command.name),
                "'{}' is documented as built-in but can be injected over",
                command.name
            );
        }
    }
}
