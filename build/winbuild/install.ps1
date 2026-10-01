#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Installs baller.exe.

.DESCRIPTION
    Convenience wrapper. The install logic lives in build.ps1 so that this
    script and `build.ps1 -Command install` can never disagree about where the
    binary goes; delegating is the whole point of keeping this file.
#>
[CmdletBinding()]
param(
    [string]$Target = ''
)

& "$PSScriptRoot\build.ps1" -Command install -Target $Target
