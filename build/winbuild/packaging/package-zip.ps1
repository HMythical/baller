#!/usr/bin/env pwsh
[CmdletBinding()]

param(
    [Parameter(Position=0)]
    [string[]]$Paths
)

$script:ProjectRoot = Resolve-Path "$PSScriptRoot\..\..\.."
$script:BallerVersion = if ($env:BALLER_VERSION) {
    $env:BALLER_VERSION
} else {
    $cargoToml = Get-Content "$ProjectRoot\Cargo.toml"
    $versionLine = $cargoToml | Select-String '^version\s*=\s*"(.+)"'
    $versionLine.Matches.Groups[1].Value
}

if (-not $Paths) {
    $releasePath = "$ProjectRoot\target\release\baller.exe"
    if (Test-Path $releasePath) {
        $Paths = @($releasePath)
    } else {
        Write-Error "No release binary found. Run 'build.ps1 release' first."
        exit 1
    }
}

$versions = [System.Collections.Generic.List[string]]::New()

$Paths | ForEach-Object {
    if (Test-Path $_) {
        $versions.Add($_)
        Write-Host "Found release binary: $_" -ForegroundColor Green
    } else {
        Write-Warning "Path not found: $_"
    }
}

$zipFileName = "baller-$BallerVersion.zip"
if (Test-Path "$PSScriptRoot\dist\$zipFileName") {
    Remove-Item "$PSScriptRoot\dist\$zipFileName" -Force
}

$packageDir = "$PSScriptRoot\packaging\tmp"
if (Test-Path $packageDir) {
    Remove-Item $packageDir -Recurse -Force
}
New-Item -ItemType Directory -Path $packageDir

$versions | ForEach-Object {
    $binaryName = Split-Path $_ -Leaf
    Copy-Item $_ "$packageDir\$binaryName"
}

$zipPath = "$PSScriptRoot\dist\$zipFileName"
Compress-Archive -Path "$packageDir\*" -DestinationPath $zipPath -Force

if (Test-Path $packageDir) {
    Remove-Item $packageDir -Recurse -Force
}

Write-Host "Created ZIP package: $zipPath" -ForegroundColor Green

Compress-Archive -Path "$packageDir\*" -DestinationPath "$PSScriptRoot\dist\baller.zip" -Force
Remove-Item $packageDir -Recurse -Force