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
    cargo build --release
    if (-not (Test-Path $OutputDir)) { New-Item -ItemType Directory -Path $OutputDir -Force }
    Copy-Item "$ProjectRoot\target\release\baller.exe" "$OutputDir\baller.exe"
    Write-Host "Binary at: $OutputDir\baller.exe" -ForegroundColor Cyan
}

function Invoke-Test {
    Write-Host "Running tests..." -ForegroundColor Green
    cargo test
    if ($LASTEXITCODE -ne 0) { throw "Tests failed" }
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
    $releaseDir = "$ProjectRoot\target\release"
    if (-not (Test-Path $releaseDir\baller.exe)) { Invoke-Release }
    if (-not (Test-Path $InstallDir)) { New-Item -ItemType Directory -Path $InstallDir -Force }
    Copy-Item "$releaseDir\baller.exe" "$InstallDir\baller.exe"
    Write-Host "Installed baller.exe to $InstallDir" -ForegroundColor Green
}

function Invoke-Uninstall {
    $path = "$InstallDir\baller.exe"
    if (Test-Path $path) { Remove-Item $path -Force; Write-Host "Removed $path" -ForegroundColor Yellow }
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
