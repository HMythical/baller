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

function Invoke-Dev {
    Write-Host "Building (debug)..." -ForegroundColor Green
    cargo build
}

function Invoke-Release {
    Write-Host "Building (release)..." -ForegroundColor Green
    $targetFlag = if ($Target) { "--target $Target" } else { "" }
    cargo build --release $targetFlag
    if (-not (Test-Path $OutputDir)) { New-Item -ItemType Directory -Path $OutputDir -Force }
    $binaryPath = if ($Target) { "$ProjectRoot\target\$Target\release\baller.exe" } else { "$ProjectRoot\target\release\baller.exe" }
    Copy-Item $binaryPath "$OutputDir\baller.exe"
    Write-Host "Binary at: $OutputDir\baller.exe" -ForegroundColor Cyan
}

function Invoke-Test {
    Write-Host "Running tests..." -ForegroundColor Green
    cargo test
    if ($LASTEXITCODE -ne 0) { throw "Tests failed" }

    Write-Host "Running clippy..." -ForegroundColor Green
    cargo clippy --all-targets -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw "Clippy failed" }

    Write-Host "Checking formatting..." -ForegroundColor Green
    cargo fmt --check
    if ($LASTEXITCODE -ne 0) { throw "Formatting check failed" }
}

function Invoke-Clean {
    Write-Host "Cleaning..." -ForegroundColor Yellow
    cargo clean
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
