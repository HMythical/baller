#!/usr/bin/env pwsh
[CmdletBinding()]

param(
    [Parameter(Position=0)]
    [ValidateSet('dev', 'release', 'test', 'clean', 'dist', 'install', 'uninstall', 'help')]
    [string]$Command = 'help',

    [switch]$Msi,
    [switch]$Exe,
    [string]$Target = ''
)

. "$PSScriptRoot\config.ps1"

# Cargo finds its project (and rustup its toolchain file) by walking up from
# the working directory, not from this script. Run from anywhere else, a bare
# `cargo` either finds no project or finds some other one — and `clean` then
# wipes that project's target\. Always run from the repo, and stop on a failing
# cargo: PowerShell ignores a native command's exit code and carries on.
#
# Arguments come as one array, never as loose words: forwarded through `$args`,
# an unquoted `--` is swallowed as PowerShell's own end-of-parameters marker.
function Invoke-Cargo {
    param([string[]]$CargoArgs)

    Push-Location $ProjectRoot
    try {
        cargo @CargoArgs
        if ($LASTEXITCODE -ne 0) { throw "cargo $($CargoArgs -join ' ') failed (exit code $LASTEXITCODE)" }
    } finally {
        Pop-Location
    }
}

function Invoke-Dev {
    Write-Host "Building (debug)..." -ForegroundColor Green
    Invoke-Cargo @('build')
}

function Invoke-Release {
    Write-Host "Building (release)..." -ForegroundColor Green
    # Each flag and its value are separate arguments. A single "--target <triple>"
    # string reaches cargo as one unknown argument, and an empty one as `''` —
    # which is how the Windows release job failed with no -Target at all.
    $cargoArgs = @('build', '--release')
    if ($Target) { $cargoArgs += @('--target', $Target) }
    Invoke-Cargo $cargoArgs
    if (-not (Test-Path $OutputDir)) { New-Item -ItemType Directory -Path $OutputDir -Force }
    $binaryPath = if ($Target) { "$ProjectRoot\target\$Target\release\baller.exe" } else { "$ProjectRoot\target\release\baller.exe" }
    Copy-Item $binaryPath "$OutputDir\baller.exe"
    Write-Host "Binary at: $OutputDir\baller.exe" -ForegroundColor Cyan
}

function Invoke-Test {
    Write-Host "Running tests..." -ForegroundColor Green
    Invoke-Cargo @('test')

    Write-Host "Running clippy..." -ForegroundColor Green
    Invoke-Cargo @('clippy', '--all-targets', '--', '-D', 'warnings')

    Write-Host "Checking formatting..." -ForegroundColor Green
    Invoke-Cargo @('fmt', '--check')
}

function Invoke-Clean {
    Write-Host "Cleaning..." -ForegroundColor Yellow
    Invoke-Cargo @('clean')
    if (Test-Path $OutputDir) { Remove-Item -Recurse -Force $OutputDir }
}

function Invoke-Dist {
    Invoke-Release
    if ($Msi) { & "$PSScriptRoot\packaging\package-msi.ps1" }
    elseif ($Exe) { & "$PSScriptRoot\packaging\package-exe.ps1" }
    else { & "$PSScriptRoot\packaging\package-zip.ps1" }
}

function Invoke-Install {
    $releaseDir = if ($Target) { "$ProjectRoot\target\$Target\release" } else { "$ProjectRoot\target\release" }
    if (-not (Test-Path $releaseDir\baller.exe)) { Invoke-Release }
    $source = "$releaseDir\baller.exe"
    if (-not (Test-Path $source)) { throw "No release binary at $source" }

    # A failure per directory is a privilege problem, not a build problem: the
    # per-machine directory needs elevation, the per-user one never does. So
    # install to whichever we can and name the ones we could not, rather than
    # letting one unwritable directory abandon the rest.
    $installed = 0
    $skipped = @()

    foreach ($dir in $InstallDirs) {
        try {
            if (-not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir -Force -ErrorAction Stop | Out-Null }
            Copy-Item $source "$dir\baller.exe" -Force -ErrorAction Stop
            $installed++
            Write-Host "Installed $dir\baller.exe" -ForegroundColor Green
        } catch {
            $skipped += $dir
        }
    }

    if ($skipped.Count -gt 0) {
        Write-Host ""
        Write-Host "Skipped (needs elevated privileges): $($skipped -join ', ')" -ForegroundColor Yellow
        if ($installed -eq 0) { throw "Nothing was installed" }
        Write-Host "Re-run from an elevated PowerShell to install per-machine as well." -ForegroundColor Yellow
    }
}

function Invoke-Uninstall {
    $removed = 0
    $skipped = @()

    foreach ($dir in $InstallDirs) {
        $path = "$dir\baller.exe"
        if (Test-Path $path) {
            try {
                Remove-Item $path -Force -ErrorAction Stop
                $removed++
                Write-Host "Removed $path" -ForegroundColor Yellow
            } catch {
                $skipped += $path
            }
        }
    }

    if ($skipped.Count -gt 0) {
        Write-Host "Skipped (needs elevated privileges): $($skipped -join ', ')" -ForegroundColor Yellow
    }
    if ($removed -eq 0) {
        Write-Host "No baller.exe found in: $($InstallDirs -join ', ')" -ForegroundColor Gray
        return
    }
    Write-Host "Uninstalled successfully" -ForegroundColor Green
}

switch ($Command) {
    'dev'       { Invoke-Dev }
    'release'   { Invoke-Release }
    'test'      { Invoke-Test }
    'clean'     { Invoke-Clean }
    'dist'      { Invoke-Dist }
    'install'   { Invoke-Install }
    'uninstall' { Invoke-Uninstall }
    'help'      { Get-Help $PSCommandPath }
}
