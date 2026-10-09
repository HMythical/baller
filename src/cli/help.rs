use colored::Colorize;
use serde::Serialize;
use serde_json::{json, Value};

use crate::context::GlobalFlags;
use crate::core::injected::{load_injected, InjectedCommand};
use crate::error::error::BallError;
use crate::utils::output::print_json;

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
    ///
    /// An indented line continues the explanation above it, and a
    /// `Subcommands:` line starts the subcommand list: `--json` relies on both
    /// to split the lines back into rows.
    details: &'static [&'static str],
    /// Free-form paragraphs shown under the arguments. A `\n` inside one is a
    /// line break in the text help and a space in `--json`.
    notes: &'static [&'static str],
}

const BUILTIN_COMMANDS: [BuiltinCommand; 12] = [
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
        notes: &["A Cargo project's dependencies are resolved by cargo itself, so\n\
            --source and --no-deps do not apply to one."],
    },
    BuiltinCommand {
        name: "inject",
        summary: "Injects a custom command described by a .ball file",
        usage: "baller inject <PATH>",
        details: &["<PATH>  .ball file describing the command to add"],
        notes: &[
            "Injecting takes three confirmations: the named binary then runs as\n\
             'baller <command-name>' with your privileges.",
            "Injected commands are stored in injected_commands.json under baller's\n\
             config directory, and are listed at the bottom of 'baller help'.",
        ],
    },
    BuiltinCommand {
        name: "referee",
        summary: "Referees (Audits) the roster against public vulnerability data",
        usage: "baller referee [PACKAGE_NAME] | baller referee <SUBCOMMAND>",
        details: &[
            "[PACKAGE_NAME]   Package to audit; omit to audit the whole roster (same as 'audit')",
            "--refresh        Re-query the advisory service instead of reusing cached verdicts",
            "--no-scan        Skip the artifact re-scan and only check advisory data",
            "",
            "Subcommands:",
            "audit [PKG...]   Advisory check and artifact re-scan (the default)",
            "                 --refresh, --no-scan, --fail-on <block|warn>,",
            "                 --format <json|markdown|sarif>, --out <FILE>",
            "check [PKG...]   Advisory data only: --refresh, --fail-on <block|warn>",
            "scan [PKG...]    Artifact re-scan only: --fail-on <block|warn>",
            "cache            --status (default) | --clear | --prune <DAYS> [--include-vulnerable]",
            "                 --prune keeps vulnerable verdicts unless --include-vulnerable",
            "config           Print the [referee] settings in effect; the VirusTotal key shows as set/unset",
            "sbom             CycloneDX 1.5 JSON of the roster: --out <FILE>, --format cyclonedx-json",
        ],
        notes: &[
            "Every subcommand is read-only: a flagged package stays installed until\n\
             you eject or update it yourself. --fail-on only changes the exit code\n\
             (1 when an advisory or a re-scan finding reaches the level), for CI.",
            "'cache' writes only to the verdict cache; 'cache', 'config' and 'sbom'\n\
             work with Referee off.",
            "Referee also runs automatically before draft, update, substitute and\n\
             build install anything. Pass --no-referee to skip it for one command, or\n\
             set 'enabled = false' under [referee] in baller.conf to turn it off.",
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

const GLOBAL_OPTIONS: [&str; 10] = [
    "-y, --yes          Skip confirmation prompts",
    "-q, --quiet        Suppress progress bars and step-by-step output",
    "-v, --verbose      Increase output detail",
    "--json             Emit machine-readable JSON instead of formatted text",
    "--no-hooks         Skip every pre/post install, eject and update hook",
    "--no-color         Disable colored output",
    "--config <DIR>     Use an alternate baller directory",
    "--no-referee       Skip the Referee security checks for this run",
    "-h, --help         Print a short usage summary",
    "-V, --version      Print baller's version",
];

/// Prints the command overview, or the details of a single command.
///
/// `topic` resolves against built-ins first, then injected commands, so a
/// built-in can never be shadowed in help output. Under `--json` the same
/// content is printed as one JSON document instead.
pub fn execute_command_help(
    baller_dir: &str,
    topic: Option<&str>,
    flags: &GlobalFlags,
) -> Result<(), BallError> {
    let injected = load_injected(baller_dir);

    let Some(name) = topic else {
        if flags.json {
            return print_json(&overview_json(&injected));
        }
        println!("{}", render_overview(&injected));
        return Ok(());
    };

    if let Some(builtin) = find_builtin(name) {
        if flags.json {
            return print_json(&builtin_detail_json(builtin));
        }
        println!("{}", render_builtin_detail(builtin));
        return Ok(());
    }

    match injected.iter().find(|c| c.command_name == name) {
        Some(command) => {
            if flags.json {
                return print_json(&injected_detail_json(command));
            }
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

/// One `name  explanation` line of the help text, split for `--json`.
#[derive(Debug, Serialize)]
struct HelpRow {
    name: String,
    description: String,
}

/// Split preformatted help lines into `(arguments, subcommands)` rows.
///
/// The name ends at the first double space. An indented line is a wrapped
/// explanation and joins the row above; `Subcommands:` moves every later row
/// into the second list; blank lines only separate.
fn split_rows(lines: &[&str]) -> (Vec<HelpRow>, Vec<HelpRow>) {
    let mut arguments: Vec<HelpRow> = Vec::new();
    let mut subcommands: Vec<HelpRow> = Vec::new();
    let mut in_subcommands = false;

    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        if *line == "Subcommands:" {
            in_subcommands = true;
            continue;
        }

        let rows = if in_subcommands {
            &mut subcommands
        } else {
            &mut arguments
        };

        if line.starts_with(' ') {
            if let Some(row) = rows.last_mut() {
                row.description.push(' ');
                row.description.push_str(line.trim());
            }
            continue;
        }

        let (name, description) = line.split_once("  ").unwrap_or((line, ""));
        rows.push(HelpRow {
            name: name.trim().to_string(),
            description: description.trim().to_string(),
        });
    }

    (arguments, subcommands)
}

/// `baller --json help`: every command, built-in and injected.
fn overview_json(injected: &[InjectedCommand]) -> Value {
    let commands: Vec<Value> = BUILTIN_COMMANDS
        .iter()
        .map(|c| {
            json!({
                "name": c.name,
                "summary": c.summary,
                "usage": c.usage,
            })
        })
        .collect();

    let injected: Vec<Value> = injected
        .iter()
        .map(|c| {
            json!({
                "name": c.command_name,
                "description": c.description,
                "version": c.version,
            })
        })
        .collect();

    json!({
        "command": "help",
        "about": TAGLINE,
        "usage": "baller <COMMAND> [ARGS]...",
        "commands": commands,
        "injected": injected,
        "global_options": split_rows(&GLOBAL_OPTIONS).0,
    })
}

/// `baller --json help <builtin>`.
fn builtin_detail_json(command: &BuiltinCommand) -> Value {
    let (arguments, subcommands) = split_rows(command.details);
    let notes: Vec<String> = command
        .notes
        .iter()
        .map(|note| note.replace('\n', " "))
        .collect();

    json!({
        "command": "help",
        "topic": command.name,
        "kind": "builtin",
        "summary": command.summary,
        "usage": command.usage,
        "arguments": arguments,
        "subcommands": subcommands,
        "notes": notes,
    })
}

/// `baller --json help <injected>`: every field of its `.ball` manifest.
fn injected_detail_json(command: &InjectedCommand) -> Value {
    json!({
        "command": "help",
        "topic": command.command_name,
        "kind": "injected",
        "description": command.description,
        "usage": format!("baller {} [ARGS]...", command.command_name),
        "version": command.version,
        "author": command.author,
        "flags": command.flags,
        "depends": command.depends,
        "require_root": command.require_root,
        "binary": command.path.display().to_string(),
    })
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
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A directory no other test can be looking at.
    ///
    /// `tag` is a label for the human reading a failure; uniqueness comes from
    /// the counter, because two tests that pick the same tag would otherwise
    /// share a path and delete each other's files mid-run.
    fn test_dir(tag: &str) -> String {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join("baller_help_tests").join(format!(
            "{}_{}_{}",
            std::process::id(),
            tag,
            n
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.to_string_lossy().to_string()
    }

    fn text() -> GlobalFlags {
        GlobalFlags::default()
    }

    fn json_flags() -> GlobalFlags {
        GlobalFlags {
            json: true,
            ..GlobalFlags::default()
        }
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
        assert!(execute_command_help(&dir, None, &text()).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_help_topic_resolves_builtin() {
        let dir = test_dir("builtin_topic");
        for command in BUILTIN_COMMANDS.iter() {
            assert!(
                execute_command_help(&dir, Some(command.name), &text()).is_ok(),
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
        assert!(execute_command_help(&dir, Some("my-tool"), &text()).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_help_topic_unknown_is_an_error() {
        let dir = test_dir("unknown_topic");
        let err = execute_command_help(&dir, Some("not-a-command"), &text()).unwrap_err();
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
        assert!(execute_command_help(&dir, Some("draft"), &text()).is_ok());
        let detail = render_builtin_detail(find_builtin("draft").unwrap());
        assert!(detail.contains("<PACKAGE_NAME>"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_help_json_runs_for_overview_and_every_topic() {
        let dir = test_dir("json_topics");
        save_injected(&dir, &[sample()]).unwrap();
        assert!(execute_command_help(&dir, None, &json_flags()).is_ok());
        for command in BUILTIN_COMMANDS.iter() {
            assert!(execute_command_help(&dir, Some(command.name), &json_flags()).is_ok());
        }
        assert!(execute_command_help(&dir, Some("my-tool"), &json_flags()).is_ok());
        assert!(matches!(
            execute_command_help(&dir, Some("not-a-command"), &json_flags()),
            Err(BallError::UnsupportedCommand(_))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_overview_json_lists_builtins_injected_and_global_options() {
        let value = overview_json(&[sample()]);
        assert_eq!(value["command"], "help");

        let commands = value["commands"].as_array().unwrap();
        assert_eq!(commands.len(), BUILTIN_COMMANDS.len());
        assert_eq!(commands[0]["name"], "draft");
        assert_eq!(commands[0]["usage"], "baller draft <PACKAGE_NAME>");

        assert_eq!(value["injected"][0]["name"], "my-tool");
        assert_eq!(
            value["injected"][0]["description"],
            "A helpful tool that does X"
        );
        assert_eq!(value["injected"][0]["version"], "1.0.0");

        let options = value["global_options"].as_array().unwrap();
        assert_eq!(options.len(), GLOBAL_OPTIONS.len());
        assert_eq!(options[0]["name"], "-y, --yes");
        assert_eq!(options[0]["description"], "Skip confirmation prompts");
        assert!(options.iter().any(|o| o["name"] == "--config <DIR>"));
    }

    #[test]
    fn test_overview_json_with_no_injected_commands_is_an_empty_list() {
        let value = overview_json(&[]);
        assert_eq!(value["injected"], json!([]));
    }

    #[test]
    fn test_builtin_detail_json_splits_arguments() {
        let value = builtin_detail_json(find_builtin("eject").unwrap());
        assert_eq!(value["topic"], "eject");
        assert_eq!(value["kind"], "builtin");
        assert_eq!(value["usage"], "baller eject <PACKAGE_NAME>");
        assert_eq!(value["arguments"][1]["name"], "-f, --force");
        assert_eq!(
            value["arguments"][1]["description"],
            "Eject even when the package is frozen"
        );
        assert_eq!(value["subcommands"], json!([]));
        assert_eq!(
            value["notes"],
            json!(["Frozen packages must be unfrozen before they can be ejected."])
        );
    }

    #[test]
    fn test_builtin_detail_json_folds_wrapped_lines() {
        let value = builtin_detail_json(find_builtin("build").unwrap());
        let path = &value["arguments"][0];
        assert_eq!(path["name"], "<PATH>");
        assert_eq!(
            path["description"],
            "Manifest (.toml or .json) to build from, or a directory \
             containing a Cargo project (Rust sources are compiled \
             with 'cargo build --release' and installed)"
        );
        assert_eq!(
            value["notes"],
            json!([
                "A Cargo project's dependencies are resolved by cargo itself, so \
                    --source and --no-deps do not apply to one."
            ])
        );
    }

    #[test]
    fn test_referee_detail_json_lists_its_subcommands() {
        let value = builtin_detail_json(find_builtin("referee").unwrap());
        let arguments = value["arguments"].as_array().unwrap();
        assert_eq!(arguments.len(), 3);
        assert_eq!(arguments[0]["name"], "[PACKAGE_NAME]");

        let subcommands = value["subcommands"].as_array().unwrap();
        let names: Vec<&str> = subcommands
            .iter()
            .map(|s| {
                s["name"]
                    .as_str()
                    .unwrap()
                    .split_whitespace()
                    .next()
                    .unwrap()
            })
            .collect();
        assert_eq!(names, ["audit", "check", "scan", "cache", "config", "sbom"]);
        assert!(subcommands[0]["description"]
            .as_str()
            .unwrap()
            .ends_with("--format <json|markdown|sarif>, --out <FILE>"));
        assert_eq!(value["notes"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn test_every_help_row_has_a_name_and_a_description() {
        for command in BUILTIN_COMMANDS.iter() {
            let (arguments, subcommands) = split_rows(command.details);
            for row in arguments.iter().chain(subcommands.iter()) {
                assert!(
                    !row.name.is_empty() && !row.description.is_empty(),
                    "'{}' has a malformed help row: {:?}",
                    command.name,
                    row
                );
            }
        }
        for row in split_rows(&GLOBAL_OPTIONS).0 {
            assert!(!row.name.is_empty() && !row.description.is_empty());
        }
    }

    #[test]
    fn test_notes_render_as_lines_in_text_and_paragraphs_in_json() {
        let command = find_builtin("inject").unwrap();
        let detail = render_builtin_detail(command);
        assert!(detail.contains("runs as\n'baller <command-name>' with your privileges."));

        let value = builtin_detail_json(command);
        let notes = value["notes"].as_array().unwrap();
        assert_eq!(notes.len(), 2);
        assert!(notes.iter().all(|n| !n.as_str().unwrap().contains('\n')));
    }

    #[test]
    fn test_injected_detail_json_carries_every_manifest_field() {
        let value = injected_detail_json(&sample());
        assert_eq!(value["topic"], "my-tool");
        assert_eq!(value["kind"], "injected");
        assert_eq!(value["usage"], "baller my-tool [ARGS]...");
        assert_eq!(value["author"], "HMythical");
        assert_eq!(value["flags"], json!(["-y", "--yes"]));
        assert_eq!(value["depends"], json!(["python3"]));
        assert_eq!(value["require_root"], true);
        assert_eq!(value["binary"], "/usr/local/bin/my-tool");
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
    fn test_every_referee_subcommand_is_documented() {
        let command = BallerCommand::command();
        let referee = command
            .find_subcommand("referee")
            .expect("referee is a subcommand");
        let entry = find_builtin("referee").unwrap();
        for subcommand in referee.get_subcommands() {
            let name = subcommand.get_name();
            assert!(
                entry
                    .details
                    .iter()
                    .any(|line| line.split_whitespace().next() == Some(name)),
                "'referee {}' is a subcommand but is not described in the referee help entry",
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
