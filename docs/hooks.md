# Hook System

Baller can execute user-defined scripts before and after package operations.
Hooks allow you to run custom logic — notify services, log events, update indexes,
or abort operations based on conditions.

## Script Location

Hook scripts live in `~/.baller/hooks/`.

| Platform | Extension | Interpreter |
|---|---|---|
| Linux | `.sh` | `bash <script>` |
| Windows | `.ps1` | `powershell -NoProfile -ExecutionPolicy Bypass -File <script>` |

On Windows, hooks run with `-ExecutionPolicy Bypass`, so they work on a stock
install, whose effective PowerShell policy (`Restricted`) refuses every script
file. Placing a script in your hooks directory is what opts it in — the same
consent `bash <script>` relies on — and that includes a hook you downloaded.
`-NoProfile` keeps your PowerShell profile out of hook runs, as non-interactive
bash skips your rc files. A policy enforced by **Group Policy** still overrides
`Bypass`; PowerShell then prints its own "running scripts is disabled" error,
and the hook counts as failed.

## Naming Convention

```
<package_name>_<hook_type>.sh
<package_name>_<hook_type>.ps1
```

## Hook Types

| Type | Runs | Aborts If Fails |
|---|---|---|
| `pre_install` | Before the package is downloaded (or handed to the native/cargo installer) | Yes |
| `post_install` | After the binary is linked and the package recorded | No — logged as a warning |
| `pre_eject` | Before the link is removed and the record deleted | Yes |
| `post_eject` | After the package is fully removed: link, record, extract dir and (with `--purge`) archive | No — logged as a warning |
| `pre_update` | Before the new version is downloaded | Yes |
| `post_update` | After the roster records the new version | No — logged as a warning |

## Examples

**`~/.baller/hooks/ripgrep_pre_install.sh:**
```bash
#!/bin/bash
echo "Installing ripgrep $(echo $BALLER_PACKAGE_VERSION) via baller"
# Return non-zero to abort the installation
```

**`~/.baller/hooks/fd_post_install.sh:**
```bash
#!/bin/bash
echo "fd $BALLER_PACKAGE_VERSION installed at $BALLER_INSTALL_PATH"
```

## Environment Variables

All hooks receive the following environment variables:

| Variable | Description | Available In |
|---|---|---|
| `BALLER_PACKAGE_NAME` | Package name | All hooks |
| `BALLER_PACKAGE_VERSION` | Version being installed/removed | All hooks |
| `BALLER_HOOK_TYPE` | Hook type string (e.g. `pre_install`) | All hooks |
| `BALLER_INSTALL_PATH` | Extraction/install directory | `post_install` |
| `BALLER_BIN_PATH` | Path to the binary file | `post_install` |
| `BALLER_NEW_VERSION` | Target version for update | `pre_update` |
| `BALLER_OLD_VERSION` | Previous version after update | `post_update` |

## Failing Hooks

**Pre-hooks** (`pre_install`, `pre_eject`, `pre_update`) can abort the operation
by exiting with a non-zero status code. Nothing has been changed yet, so the
operation is cancelled and the command fails:

```
[Error]: hook 'ripgrep_pre_install.sh' failed with exit code 1
```

**Post-hooks** (`post_install`, `post_eject`, `post_update`) run after the
operation has already been applied, so they cannot undo it. A failing post-hook
— a non-zero exit, or a script that cannot be started — is reported as a
warning on stderr and the command still succeeds (exit 0):

```
hook 'fd_post_install.sh' failed with exit code 2 — a failing post_install hook does not undo the install of 'fd', which completed
```

Like all progress output, the warning is hidden by `--quiet` and `--json`.

## Configuration

Hook execution can be controlled per-type in `baller.conf`:

```ini
[hooks]
pre_install = true
post_install = true
pre_eject = true
post_eject = true
pre_update = true
post_update = true
```

Set any type to `false` to disable hook execution for that type globally.
If no hook script exists for a package+type pair, the hook is silently skipped.
