# Hook System

Baller can execute user-defined scripts before and after package operations.
Hooks allow you to run custom logic — notify services, log events, update indexes,
or abort operations based on conditions.

## Script Location

Hook scripts live in `~/.baller/hooks/`.

| Platform | Extension | Interpreter |
|---|---|---|
| Linux | `.sh` | `bash <script>` |
| Windows | `.ps1` | `powershell -File <script>` |

## Naming Convention

```
<package_name>_<hook_type>.sh
<package_name>_<hook_type>.ps1
```

## Hook Types

| Type | Runs Before | Aborts If Fails |
|---|---|---|
| `pre_install` | Package download begins | Yes |
| `post_install` | DB insert and symlink complete | No (logged) |
| `pre_eject` | Symlink removal and DB delete | Yes |
| `post_eject` | Package fully removed | No (logged) |
| `pre_update` | Download of new version begins | Yes |
| `post_update` | DB updated to new version | No (logged) |

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

## Aborting Operations

Pre-hooks (`pre_install`, `pre_eject`, `pre_update`) can abort the operation by
exiting with a non-zero status code. The operation is cancelled immediately and
the error is reported to the user.

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
