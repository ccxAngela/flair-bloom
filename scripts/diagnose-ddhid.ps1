# Compatibility entry point. Produces the unified environment report.
[CmdletBinding()]
param([string]$DriverDir = '', [string]$OutFile = '', [switch]$EvidenceOnly)
& (Join-Path $PSScriptRoot 'diagnose-environment.ps1') @PSBoundParameters
