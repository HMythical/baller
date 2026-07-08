#!/usr/bin/env pwsh
[CmdletBinding()]

. "$PSScriptRoot\config.ps1"

function Invoke-Uninstall {
    $path = "$InstallDir\baller.exe"
    if (Test-Path $path) {
        Remove-Item $path -Force
        Write-Host "Removed $path" -ForegroundColor Yellow
    } else {
        Write-Host "Binary not found at $path" -ForegroundColor Gray
    }
}

Invoke-Uninstall
