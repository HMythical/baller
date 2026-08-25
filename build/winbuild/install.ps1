#!/usr/bin/env pwsh
[CmdletBinding()]
param(
    [string]$Target = ''
)

. "$PSScriptRoot\config.ps1"

function Invoke-Install {
    $releaseDir = if ($Target) { "$ProjectRoot\target\$Target\release" } else { "$ProjectRoot\target\release" }
    if (-not (Test-Path $releaseDir\baller.exe)) {
        Write-Host "Binary not found. Running release build first..." -ForegroundColor Yellow
        $targetFlag = if ($Target) { "--target $Target" } else { "" }
        cargo build --release $targetFlag
    }
    if (-not (Test-Path $InstallDir)) { New-Item -ItemType Directory -Path $InstallDir -Force }
    Copy-Item "$releaseDir\baller.exe" "$InstallDir\baller.exe"
    Write-Host "Installed baller.exe to $InstallDir" -ForegroundColor Green
}

Invoke-Install
