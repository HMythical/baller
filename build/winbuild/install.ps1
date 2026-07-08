#!/usr/bin/env pwsh
[CmdletBinding()]

. "$PSScriptRoot\config.ps1"

function Invoke-Install {
    $releaseDir = "$ProjectRoot\target\release"
    if (-not (Test-Path $releaseDir\baller.exe)) {
        Write-Host "Binary not found. Running release build first..." -ForegroundColor Yellow
        cargo build --release
    }
    if (-not (Test-Path $InstallDir)) { New-Item -ItemType Directory -Path $InstallDir -Force }
    Copy-Item "$releaseDir\baller.exe" "$InstallDir\baller.exe"
    Write-Host "Installed baller.exe to $InstallDir" -ForegroundColor Green
}

Invoke-Install
