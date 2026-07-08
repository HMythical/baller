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

$wxsFile = "$PSScriptRoot\packaging\installer.wxs"
if (-not (Test-Path $wxsFile)) {
    Write-Warning "WiX source file not found at $wxsFile. Creating a basic one..."
    Create-BasicWiXFile -PackageDir $packageDir -WxsFile $wxsFile -Version $BallerVersion
}

$wixPath = "$env:ProgramData\WiX\wix310.msi"
if (-not (Test-Path $wixPath)) {
    Write-Warning "WiX not found in $wixPath. Please install WiX (WiX 3.10 or later)."
    exit 1
}

& "$wixPath" -nologo -out "$PSScriptRoot\dist\baller-$BallerVersion.msi" $wxsFile

if (Test-Path $packageDir) {
    Remove-Item $packageDir -Recurse -Force
}

Write-Host "Created MSI package: $PSScriptRoot\dist\baller-$BallerVersion.msi" -ForegroundColor Green