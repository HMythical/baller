#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Uninstalls baller.exe.

.DESCRIPTION
    Convenience wrapper. The uninstall logic lives in build.ps1 so that this
    script and `build.ps1 -Command uninstall` can never disagree about where
    the binary was put; delegating is the whole point of keeping this file.
#>
[CmdletBinding()]
param()

& "$PSScriptRoot\build.ps1" -Command uninstall
