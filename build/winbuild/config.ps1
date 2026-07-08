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
$script:InstallDir = if ($env:BALLER_INSTALL_DIR) {
    $env:BALLER_INSTALL_DIR
} else {
    "$env:LOCALAPPDATA\baller\bin"
}