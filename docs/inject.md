# Inject — Custom Commands

The `inject` command lets you register external binaries as baller subcommands.
Once injected, `baller <command-name>` runs that binary, forwarding every
argument and the child's exit code.

## Quick Start

```
$ baller inject ./my-tool.ball
```

## How It Works

1. Baller parses the `.ball` file and validates it.
2. You confirm three sequential warnings (see [Safety Prompts](#safety-prompts)).
3. The command definition is written to `~/.baller/injected_commands.json`.
4. `baller <command-name>` now runs the binary at the path you specified.

The binary runs with **your user privileges**. Baller does not sandbox, isolate,
or restrict it in any way.

## `.ball` File Format

A `.ball` file uses `[SECTION]` headers with `KEY = VALUE` entries, similar to
INI but with bracketed section names. `#` and `;` start comments.

### Required Sections

| Section | Key | Description |
|---------|-----|-------------|
| `[COMMAND-NAME]` | `COMMAND-NAME` | The subcommand name (e.g. `my-tool` → `baller my-tool`) |
| `[PATH]` | `PATH` | Absolute or relative path to the executable |

### Optional Sections

| Section | Key | Default | Description |
|---------|-----|---------|-------------|
| `[DESCRIPTION]` | `DESCRIPTION` | `""` | Shown in `baller help` output |
| `[VERSION]` | `VERSION` | `"0.1.0"` | Version of the injected command |
| `[FLAGS]` | `FLAGS-LIST` | `[]` | Comma-separated flags for help display |
| `[AUTHOR]` | `AUTHOR` | `""` | Shown in `baller help` output |
| `[REQUIRE-ROOT]` | `ROOTPERMS` | `false` | If `true`, the command must be run as root |
| `[DEPENDS]` | `DEPENDS` | `[]` | Comma-separated tools that must exist on `PATH` |

### Value Rules

- Values may be **quoted** (`"my-tool"`) or **bare** (`my-tool`).
- `FLAGS-LIST` and `DEPENDS` are **comma-separated** on a single line.
- `ROOTPERMS` accepts `true`/`false`, `yes`/`no`, `1`/`0`, `on`/`off`.
- Section and key names are **case-insensitive**.
- Blank lines and comment lines are ignored.

### Full Example

```
# my-tool.ball
# A tool that compresses files using LZ4

[COMMAND-NAME]
COMMAND-NAME = "lz4c"

[DESCRIPTION]
DESCRIPTION = "Fast LZ4 compression tool"

[VERSION]
VERSION = "2.1.0"

[FLAGS]
FLAGS-LIST = "-1, --fast, -9, --best, -o, --output"

[AUTHOR]
AUTHOR = "Jane Developer"

[REQUIRE-ROOT]
ROOTPERMS = false

[DEPENDS]
DEPENDS = "lz4, bash"

[PATH]
PATH = /usr/local/bin/lz4c
```

### Minimal Example

Only two sections are required:

```
[COMMAND-NAME]
COMMAND-NAME = "hello"

[PATH]
PATH = /usr/local/bin/hello
```

## Safety Prompts

Before any injection, baller displays **three** sequential confirmation prompts:

```
This will modify baller's runtime behavior by adding a new command. Continue? [yes/no]
Only inject .ball files from sources you trust: the binary they name runs with your privileges. Continue? [yes/no]
Final confirmation: inject 'lz4c' from /usr/local/bin/lz4c? [yes/no]
```

Answering anything other than `yes` or `y` at any prompt **aborts immediately**
without writing anything.

## Validation Rules

`inject` rejects manifests that:

- Name a **built-in command** (`draft`, `eject`, `freeze`, `roster`,
  `substitute`, `sweep`, `update`, `build`, `inject`, `help`, `version`)
- Name a command that **already exists** (overwrites the previous injection)
- Contain a command name with **whitespace**
- Point to a **non-existent binary** at `PATH`
- Point to a **directory** instead of a file at `PATH`
- Are **missing** `COMMAND-NAME` or `PATH`
- Contain **malformed lines** (no `=` sign)

## Running Injected Commands

Once injected, the command is available immediately:

```
$ baller lz4c --fast -o output.txt input.txt
```

All arguments are forwarded as-is. The child's exit code is propagated:

```
$ baller lz4c --bad-flag
lz4c: unknown option '--bad-flag'
$ echo $?
1
```

## Dependency Checking

If `[DEPENDS]` lists tools, baller checks that each one exists on `PATH`
before running the binary. Missing dependencies produce an error:

```
$ baller lz4c
[Error]: injected command error: missing dependency 'lz4' (not found on PATH)
```

## Root Requirements

If `[REQUIRE-ROOT]` is `true`, the command can only be run as root:

```
$ baller lz4c
[Error]: injected command error: 'lz4c' requires root privileges
```

## Updating an Injected Command

Re-injecting the same command name **overwrites** the previous entry:

```
$ baller inject ./lz4c-v3.ball
...
Done Injected 'lz4c' -> /usr/local/bin/lz4c (updated)
```

## Viewing Injected Commands

```
$ baller help
...
Injected commands:
  lz4c  Fast LZ4 compression tool
  hello A greeting command
```

## Removing an Injected Command

Edit `~/.baller/injected_commands.json` directly and remove the entry, or
delete the entire file to remove all injected commands:

```
$ rm ~/.baller/injected_commands.json
```

Baller will show no injected commands on the next `baller help`.

## Anti-Patterns

### DO NOT inject untrusted binaries

The binary runs with your full user privileges. Injecting a binary from an
untrusted source is equivalent to running an unknown executable from the
internet. **Baller provides no sandboxing or isolation.**

```
# BAD — do not do this
$ baller inject ./random-download.ball
```

### DO NOT use relative PATH without understanding the risk

`PATH` is canonicalized at injection time. A relative path is resolved from
the working directory at that moment. If you move the binary later, the
injection breaks.

```
# Fragile — resolved from wherever you run this
[PATH]
PATH = ./my-tool

# Better — use an absolute path
[PATH]
PATH = /home/user/bin/my-tool
```

### DO NOT use reserved command names

These names are taken by baller and cannot be overridden:

```
draft, eject, freeze, roster, substitute, sweep, update, build, inject, help, version
```

### DO NOT skip the warnings

The three-prompt flow exists because injected binaries run with your privileges.
If you automate injection (e.g. in a script), pipe `yes` only if you have
reviewed the `.ball` file and trust the binary it names.

### DO NOT inject without checking DEPENDS

If your binary requires `python3` or `ffmpeg`, declare them in `[DEPENDS]`.
Running an injected command that silently depends on missing tools produces
confusing failures.

## File Location

| Platform | Path |
|----------|------|
| Linux | `~/.baller/injected_commands.json` |
| Windows | `%LOCALAPPDATA%\baller\injected_commands.json` |

## JSON Structure

The persisted file is a JSON array of command objects:

```json
[
  {
    "command_name": "lz4c",
    "description": "Fast LZ4 compression tool",
    "version": "2.1.0",
    "flags": ["-1", "--fast", "-9", "--best", "-o", "--output"],
    "author": "Jane Developer",
    "require_root": false,
    "depends": ["lz4", "bash"],
    "path": "/usr/local/bin/lz4c"
  }
]
```

## Liability

**You are solely responsible for any binaries you inject into baller.** The
baller project, its contributors, and maintainers assume **no liability** for
damages, data loss, security breaches, or any other consequences resulting from
the use of injected commands. By using `baller inject`, you acknowledge that:

1. Injected binaries execute with **your full user privileges**.
2. You have **reviewed and trust** the binary and the `.ball` file.
3. Baller performs **no sandboxing, validation, or security scanning** of
   injected binaries.
4. **You accept all risk** associated with running third-party executables
   through baller.
5. The baller project provides injected commands **"as is"** with **no warranty
   of any kind**.

If you do not accept these terms, do not use `baller inject`.
