# Contributing to B.A.L.L.E.R.

Thank you for your interest in contributing to B.A.L.L.E.R. (**B**inary **A**llocation & **L**ibrary **L**aunch **E**nvironment in **R**ust), a cross-platform package manager for Linux and Windows.

This guide covers everything you need to submit a contribution: environment setup, branch and commit conventions, testing expectations, and the review process.

**Quick links**

| Resource | Location |
|---|---|
| Technical setup reference | [`docs/contributing.md`](docs/contributing.md) |
| Architecture overview | [`docs/architecture.md`](docs/architecture.md) |
| Command reference | [`docs/commands.md`](docs/commands.md) |
| Pull request template | [`.github/pull_request_template.md`](.github/pull_request_template.md) |
| License | [`LICENSE`](LICENSE) |
| Security contact | Discord: **HMythical** |

---

## Table of Contents

1. [Getting Started](#1-getting-started)
2. [Branch Strategy](#2-branch-strategy)
3. [Code Contributions](#3-code-contributions)
4. [Commit Style](#4-commit-style)
5. [Documentation](#5-documentation)
6. [Security Issues](#6-security-issues)
7. [Pull Request Process](#7-pull-request-process)
8. [Code of Conduct](#8-code-of-conduct)
9. [License](#9-license)
10. [Getting Help](#10-getting-help)

---

## 1. Getting Started

### Prerequisites

| Requirement | Notes |
|---|---|
| Rust (stable toolchain) | 2021 edition, installed via [rustup](https://rustup.rs) |
| Git | Any recent version |
| Linux tooling | A C toolchain and `pkg-config` for bundled dependencies |
| Windows tooling | PowerShell 5.1 or later and the MSVC build tools |

Verify your toolchain before starting:

```bash
rustc --version
cargo --version
```

### Fork and Clone

1. Fork `HMythical/baller` under your own GitHub account.
2. Clone your fork locally:

   ```bash
   git clone https://github.com/<your-username>/baller.git
   cd baller
   ```

3. Add the upstream remote so you can stay in sync:

   ```bash
   git remote add upstream https://github.com/HMythical/baller.git
   git fetch upstream
   ```

4. Confirm your commit identity is set correctly so your work is attributed to you:

   ```bash
   git config user.name "Your Name"
   git config user.email "you@example.com"
   ```

5. Create a branch off `rootdev` for your work (see [Branch Strategy](#2-branch-strategy)).

### Development Setup

**Linux**

```bash
./build/linux/build.sh dev      # Debug build
./build/linux/build.sh release  # Release build (stripped)
./build/linux/build.sh test     # Tests + clippy + fmt check
./build/linux/build.sh clean    # Clean artifacts
```

**Windows (PowerShell)**

```powershell
.\build\winbuild\build.ps1 -Command dev      # Debug build
.\build\winbuild\build.ps1 -Command release  # Release build
.\build\winbuild\build.ps1 -Command test     # Run tests
.\build\winbuild\build.ps1 -Command clean    # Clean artifacts
```

You can also use `cargo` directly. See [`docs/contributing.md`](docs/contributing.md) for the full project layout, configuration file format, and dependency list.

---

## 2. Branch Strategy

### Branch Naming Conventions

Branch names use a type prefix followed by a short, hyphenated description.

| Prefix | Use for | Example |
|---|---|---|
| `feature/` | New functionality | `feature/parallel-downloads` |
| `fix/` | Bug fixes | `fix/manifest-path-resolution` |
| `docs/` | Documentation changes | `docs/registry-configuration` |
| `refactor/` | Restructuring without behaviour change | `refactor/dep-solver-traits` |
| `test/` | Test additions or changes | `test/downloader-edge-cases` |
| `chore/` | Build, CI, and maintenance work | `chore/bump-clap-version` |

> **Note:** Keep descriptions lowercase and hyphen-separated. Avoid branch names that only reference an issue number.

### Workflow

| Branch | Purpose |
|---|---|
| `rootdev` | Main development branch. All contributions target this branch. |
| `deploy` | Release branch. Maintained by the project maintainer. |

The flow is:

```
your-fork/feature/<description>  ->  HMythical/baller:rootdev  ->  deploy (releases)
```

Contributors open pull requests against `rootdev`. Only the maintainer promotes `rootdev` to `deploy` for a release.

Keep your branch current before opening a pull request:

```bash
git fetch upstream
git rebase upstream/rootdev
```

### Pull Request Requirements

- CI must pass on both Linux and Windows.
- The description must explain what changed and why.
- New functionality must include test coverage.
- Changes should stay within a single platform where practical (see [Platform-Specific Rules](#platform-specific-rules)).

---

## 3. Code Contributions

### Language Requirements

| Language | Scope |
|---|---|
| Rust | Primary implementation language for all application code |
| PowerShell | Windows build, install, and packaging scripts |
| Bash | Linux build, install, and packaging scripts |
| Python | CI helper and validation scripts under `scripts/` |

New languages and runtimes are not accepted without prior discussion in a GitHub issue. Every dependency added to the project increases the surface area a package manager must be trusted with, so additions need justification.

### Code Style and Conventions

Formatting is enforced by `rustfmt` and linting by `clippy`. Run both before committing:

```bash
cargo fmt
cargo clippy -- -D warnings
```

Naming and structural conventions:

- Non-OS structs use PascalCase with context included: `BallerConfig`, not `Config`.
- Variables use snake_case with explicit types where inference is not obvious.
- OS-specific structs are prefixed with the OS: `LinuxPackageInfo`, `WindowsPackageInfo`.
- OS-specific variables are prefixed with `os_`.
- Configuration structs are suffixed with `Config`; builders with `Builder`; data transfer objects with `DTO`.
- Booleans are prefixed with `is_`, `has_`, `can_`, or `should_`.
- Collections and tuples use plural names.
- Errors are propagated with the `?` operator up to `main()`. Avoid `unwrap()` and `expect()` outside tests.
- Names must be unambiguous and must not collide with names from external crates.

The full convention list, including the project structure and config file format, is in [`docs/contributing.md`](docs/contributing.md).

### Testing Requirements

Every behavioural change needs a test that would fail without it.

```bash
cargo test                  # Run all tests
cargo test -- --nocapture   # Run with output
cargo test test_name        # Run a specific test
```

- Cover the normal path and the edge cases, including malformed input and failure modes.
- Tests must not depend on network access, a specific machine, or a pre-existing package database.
- Record what you tested, what you expected, and what you observed in the pull request description.

> **Warning:** B.A.L.L.E.R. installs and removes software on a user's machine. Code paths that write to the filesystem, elevate privileges, execute hooks, or verify checksums receive additional scrutiny. Do not submit changes to these paths without tests.

### Platform-Specific Rules

- All OS-specific code lives behind `#[cfg(target_os = "...")]` guards, with implementations under `src/platform/`.
- Shared behaviour is defined by the `PlatformManager` trait in `src/platform/common.rs`. Add to the trait rather than branching on the OS at call sites.
- Prefer keeping a single pull request to a single platform. A change that touches both Linux and Windows is harder to review and harder to test, and may be sent back to be split.
- Cross-platform changes to shared modules (OS detection, dependency resolution, the CLI layer) are expected to touch both and are exempt from the rule above.

---

## 4. Commit Style

### Format

```
(<type>) <description>

<body>

<footer>
```

### Types

| Type | Use for |
|---|---|
| `feat` | New feature |
| `fix` | Bug fix |
| `docs` | Documentation only |
| `style` | Formatting, no behaviour change |
| `refactor` | Restructuring without behaviour change |
| `test` | Tests |
| `chore` | Build, CI, and maintenance |

### Rules

- Use the imperative mood: "Add resolver", not "Added resolver".
- Keep the subject line under 72 characters.
- Leave a blank line between the subject, body, and footer.
- Explain *what* changed and *why* in the body. The diff already shows *how*.
- Reference related issues in the footer with `Closes #<n>` or `Refs #<n>`.
- Note test coverage and any limitations or trade-offs.

### Examples

A feature commit:

```
(feat) Add topological dependency resolver

Replace the naive recursive walk in dep_solver.rs with a topological
sort that detects cycles before installation begins. The previous
implementation could recurse indefinitely on a circular dependency
graph and left the package database in a partially written state.

Tested with a three-package cycle, a diamond dependency, and a
1,000-node synthetic graph. Cycle detection reports the full path
rather than only the repeated node.

Closes #42
```

A fix commit:

```
(fix) Reject archive entries that escape the extraction root

Archive extraction did not validate entry paths, so an archive
containing "../" components could write outside the cache directory.
Entries are now canonicalised and rejected if they resolve outside
the target root.

Refs #58
```

### Common Mistakes

| Avoid | Use instead |
|---|---|
| `fixed stuff` | `(fix) Correct cache path on Windows` |
| `(feat): Add resolver` | `(feat) Add resolver` |
| `Update main.rs` | A subject describing the behaviour that changed |
| A subject line with no body | A body explaining the motivation and testing |
| Bundling unrelated changes | One logical change per commit |

---

## 5. Documentation

### Code Documentation

- Document every public item with `///` doc comments, including what it returns and the conditions under which it errors.
- Use module-level `//!` comments to explain a module's responsibility.
- Add inline comments only where the reasoning is not obvious from the code. Explain why, not what.
- Update existing comments when you change the behaviour they describe. A stale comment is worse than none.
- Add a doc example for public APIs where a caller would benefit from seeing usage.

### Project Documentation

- User-facing changes require an update to the relevant file under `docs/`.
- New commands or flags require an update to [`docs/commands.md`](docs/commands.md).
- Changes to the manifest, registry, or hook systems require updates to their respective documents in `docs/`.
- Update [`README.md`](README.md) only when installation or build instructions change.

### Pull Request Documentation

Your pull request should stand on its own for a reviewer who has not seen the code before:

- A clear description of the change and the problem it solves.
- Test results, including what you ran and on which platform.
- Terminal output, screenshots, or a recording for changes to user-visible behaviour.
- Any breaking changes, called out explicitly.
- Known limitations or follow-up work.

---

## 6. Security Issues

### Reporting Process

> **Warning:** Do not open a public GitHub issue for a security vulnerability. Public disclosure before a fix is available puts users at risk.

Report vulnerabilities privately via Discord to **HMythical**.

Include the following in your report:

| Field | Description |
|---|---|
| Description | What the vulnerability is and which component is affected |
| Steps to reproduce | A minimal, reliable reproduction |
| Impact | What an attacker could achieve, and under what preconditions |
| Affected versions | Commit, tag, or release where you observed the issue |
| Suggested fix | Optional, but appreciated |

Please give the maintainer a reasonable opportunity to release a fix before disclosing the issue publicly.

### What Not to Report Here

| Type | Where it belongs |
|---|---|
| General bugs and crashes | GitHub Issues |
| Feature requests | GitHub Issues |
| Usage questions | GitHub Discussions |
| Build or setup problems | GitHub Discussions |

### Response Timeline

| Stage | Target |
|---|---|
| Initial acknowledgment | Within 48 hours |
| Assessment and severity triage | Within 1 week |
| Fix and release | Depends on severity and complexity |

---

## 7. Pull Request Process

### Before Submitting

Run the full local check suite and confirm every item passes:

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

Then confirm the following:

- [ ] Your branch is rebased on the latest `upstream/rootdev`.
- [ ] New functionality has tests, and all tests pass.
- [ ] Documentation is updated for any public API or user-facing change.
- [ ] Commit messages follow the format in [Commit Style](#4-commit-style).
- [ ] The change contains no unrelated edits, commented-out code, or debug output.
- [ ] No secrets, credentials, or absolute local paths are committed.

### Opening the Pull Request

Open the pull request against `rootdev`. The [pull request template](.github/pull_request_template.md) is applied automatically. Fill in every section, including:

- Description
- Type of change
- Platform tested
- Testing performed
- Checklist
- Related issues
- Screenshots or recordings, where applicable

Mark the pull request as a draft if you want early feedback on work that is not finished.

### Review Process

| Stage | Detail |
|---|---|
| CI | Linux and Windows workflows must pass before review begins |
| Approval | At least one maintainer approval is required to merge |
| Changes requested | Push additional commits to the same branch; do not force-push mid-review unless asked |
| Merge | Squash and merge, so `rootdev` keeps a linear history |

If a pull request or commit message is unclear, the maintainer will ask you to explain the change rather than guess at it. The goal is that the next contributor can read the history and understand why the code is the way it is.

### Code Provenance

Do not copy code from other projects into this repository, including from Chocolatey or any other package manager. Contribute code you wrote or that you have an unambiguous right to relicense under Apache 2.0. If a change is derived from another project, say so explicitly in the pull request so the license can be reviewed.

---

## 8. Code of Conduct

This project adopts the [Contributor Covenant Code of Conduct, version 2.1](https://www.contributor-covenant.org/version/2/1/code_of_conduct/).

### Our Pledge

We as members, contributors, and leaders pledge to make participation in our community a harassment-free experience for everyone, regardless of age, body size, visible or invisible disability, ethnicity, sex characteristics, gender identity and expression, level of experience, education, socio-economic status, nationality, personal appearance, race, caste, color, religion, or sexual identity and orientation.

We pledge to act and interact in ways that contribute to an open, welcoming, diverse, inclusive, and healthy community.

### Our Standards

Examples of behavior that contributes to a positive environment:

- Demonstrating empathy and kindness toward other people
- Being respectful of differing opinions, viewpoints, and experiences
- Giving and gracefully accepting constructive feedback
- Accepting responsibility, apologizing to those affected by our mistakes, and learning from the experience
- Focusing on what is best for the overall community, not just for us as individuals

Examples of unacceptable behavior:

- The use of sexualized language or imagery, and sexual attention or advances of any kind
- Trolling, insulting or derogatory comments, and personal or political attacks
- Public or private harassment
- Publishing others' private information, such as a physical or email address, without their explicit permission
- Other conduct which could reasonably be considered inappropriate in a professional setting

### Enforcement Responsibilities

Project maintainers are responsible for clarifying and enforcing these standards of acceptable behavior and will take appropriate and fair corrective action in response to any behavior they deem inappropriate, threatening, offensive, or harmful.

Maintainers have the right and responsibility to remove, edit, or reject comments, commits, code, issues, and other contributions that are not aligned with this Code of Conduct, and will communicate reasons for moderation decisions when appropriate.

### Scope

This Code of Conduct applies within all community spaces, including the GitHub repository, issues, discussions, pull requests, and the project Discord. It also applies when an individual is officially representing the project in public spaces.

### Enforcement

Instances of abusive, harassing, or otherwise unacceptable behavior may be reported to the project maintainer via Discord to **HMythical**. All complaints will be reviewed and investigated promptly and fairly.

Maintainers are obligated to respect the privacy and security of the reporter of any incident. Consequences for violations follow the enforcement guidelines described in the [Contributor Covenant v2.1](https://www.contributor-covenant.org/version/2/1/code_of_conduct/), ranging from a private warning to a permanent ban from the community.

### Attribution

This Code of Conduct is adapted from the [Contributor Covenant](https://www.contributor-covenant.org), version 2.1, available at https://www.contributor-covenant.org/version/2/1/code_of_conduct.html.

---

## 9. License

All contributions to B.A.L.L.E.R. are licensed under the **Apache License 2.0**.

By submitting a pull request, you agree that your contributions will be licensed under the same terms as the project, and you confirm that you have the right to license them that way.

> **Note:** No Contributor License Agreement is required. The CLA that previously applied to this project has been removed. Contributing requires nothing beyond opening a pull request.

For the full terms, see the [LICENSE](LICENSE) file.

---

## 10. Getting Help

| Need | Where to go |
|---|---|
| Report a bug | [GitHub Issues](https://github.com/HMythical/baller/issues) |
| Request a feature | [GitHub Issues](https://github.com/HMythical/baller/issues) |
| Ask a usage or design question | GitHub Discussions |
| Report a security vulnerability | Discord: **HMythical** |
| Report a Code of Conduct violation | Discord: **HMythical** |
| Read technical documentation | The [`docs/`](docs/) directory |

Before opening an issue, search existing issues to see whether it has already been reported.

---

B.A.L.L.E.R. is early in its life and there is a great deal still to build. Thank you for taking the time to contribute.
