#!/usr/bin/env pwsh
[CmdletBinding()]

$script:ProjectRoot = Resolve-Path "$PSScriptRoot\..\.."
$script:BallerVersion = if ($env:BALLER_VERSION) {
    $env:BALLER_VERSION
} else {
    $cargoToml = Get-Content "$ProjectRoot\Cargo.toml"
    $versionLine = $cargoToml | Select-String '^version\s*=\s*"(.+)"'
    $versionLine.Matches.Groups[1].Value
}
$script:BallerTarget = if ($env:BALLER_TARGET) { $env:BALLER_TARGET } else { "x86_64-pc-windows-msvc" }
$script:BallerProfile = if ($env:BALLER_PROFILE) { $env:BALLER_PROFILE } else { "debug" }
$script:OutputDir = if ($env:BALLER_OUTPUT_DIR) { $env:BALLER_OUTPUT_DIR } else { "$PSScriptRoot\dist" }

# Where `install` puts baller.exe and `uninstall` looks for it.
#
# The Windows counterparts of the Linux pair: a per-machine directory that only
# an elevated shell can write, and a per-user directory that cannot. Both can
# end up on PATH, and whichever comes first wins, so writing to only one of
# them is how a machine ends up with two baller.exe files that disagree — the
# fresh one silently shadowed by the stale one.
#
# 32-bit PowerShell on 64-bit Windows reports a redirected `ProgramFiles`, so
# prefer the 64-bit view and fall back through the remaining spellings. The last
# fallback is string interpolation rather than Join-Path: Join-Path resolves
# through the PowerShell provider, and a bare drive specifier like `C:` is not a
# provider path, so it throws. This runs in a dot-sourced config with no
# error handling, so a throw here means the whole command produces no output.
$script:ProgramFilesDir = if ($env:ProgramW6432) {
    $env:ProgramW6432
} elseif ($env:ProgramFiles) {
    $env:ProgramFiles
} else {
    "$env:SystemDrive\Program Files"
}
$script:SystemInstallDir = if ($env:BALLER_SYSTEM_INSTALL_DIR) {
    $env:BALLER_SYSTEM_INSTALL_DIR
} else {
    "$script:ProgramFilesDir\baller\bin"
}
$script:UserInstallDir = if ($env:BALLER_USER_INSTALL_DIR) {
    $env:BALLER_USER_INSTALL_DIR
} else {
    "$env:LOCALAPPDATA\baller\bin"
}

# `BALLER_INSTALL_DIR` predates the pair and still wins outright: naming one
# location is an explicit request for that location, so honour it and do not
# fan out to the others.
$script:InstallDirs = if ($env:BALLER_INSTALL_DIR) {
    @($env:BALLER_INSTALL_DIR)
} elseif ($script:SystemInstallDir -eq $script:UserInstallDir) {
    @($script:SystemInstallDir)
} else {
    @($script:SystemInstallDir, $script:UserInstallDir)
}