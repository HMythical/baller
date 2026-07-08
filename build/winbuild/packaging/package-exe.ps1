#!/usr/bin/env pwsh
[CmdletBinding()]

. "$PSScriptRoot\config.ps1"

$releasePath = "$ProjectRoot\target\release\baller.exe"
if (-not (Test-Path $releasePath)) {
    Write-Error "Release binary not found at $releasePath. Run 'build.ps1 release' first."
    exit 1
}

$packageDir = "$PSScriptRoot\packaging\tmp"
if (Test-Path $packageDir) {
    Remove-Item $packageDir -Recurse -Force
}
New-Item -ItemType Directory -Path $packageDir

Copy-Item $releasePath "$packageDir\baller.exe"
$binaryName = Split-Path $releasePath -Leaf

$nsisPath = "${env:ProgramFiles(x86)}\NSIS\makensis.exe"
if (-not (Test-Path $nsisPath)) {
    Write-Warning "NSIS not found at $nsisPath. Please install NSIS (NSIS 3.x)."
    exit 1
}

$scriptFile = "$PSScriptRoot\packaging\installer.nsi"
if (-not (Test-Path $scriptFile)) {
    Write-Warning "NSIS script file not found at $scriptFile. Creating a basic one..."
    Create-BasicNSISFile -PackageDir $packageDir -ScriptFile $scriptFile -Version $BallerVersion
}

& "$nsisPath" "$scriptFile"

if (Test-Path $packageDir) {
    Remove-Item $packageDir -Recurse -Force
}

Write-Host "Created installer package: $PSScriptRoot\dist\baller-$BallerVersion-setup.exe" -ForegroundColor Green