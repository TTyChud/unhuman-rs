<#
.SYNOPSIS
    The gate, on Windows.

.DESCRIPTION
    Mirrors scripts/gate.sh. A second copy rather than a cross-platform wrapper
    because bash on Windows runners is a Git Bash layer with its own path rules,
    and a gate that behaves differently on the platform it is meant to check is
    worse than no gate.

    The one real difference: a GitHub Windows runner has no Vulkan driver, so
    the backend suites skip. This job is about the platform -- cfg branches, path
    separators, allocation behaviour -- not about the driver. The Linux lavapipe
    job is what covers the backend.

.EXAMPLE
    .\scripts\gate.ps1 -NoVulkan
#>
[CmdletBinding()]
param(
    [switch]$NoVulkan,
    [switch]$Serial
)

$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

function Step($name) {
    Write-Host ""
    Write-Host "== $name" -ForegroundColor Cyan
}

function Invoke-Cargo($what, [string[]]$arguments) {
    Step $what
    & cargo @arguments
    if ($LASTEXITCODE -ne 0) {
        Write-Error "$what failed with exit code $LASTEXITCODE"
        exit $LASTEXITCODE
    }
}

Invoke-Cargo 'build'    @('build', '--workspace', '--all-targets')
Invoke-Cargo 'release'  @('build', '--workspace', '--release', '--locked')
Invoke-Cargo 'clippy'   @('clippy', '--workspace', '--all-targets', '--', '-D', 'warnings')
Invoke-Cargo 'format'   @('fmt', '--all', '--check')

if ($NoVulkan) {
    Step 'tests (no Vulkan driver on this runner)'
    Write-Host 'Skipping stormbreaker: a GitHub Windows runner has no Vulkan driver.'
    Write-Host 'The backend is covered by the Linux lavapipe job.'
    Invoke-Cargo 'logic tests' @(
        'test', '-p', 'grind', '-p', 'chassis', '-p', 'adam', '-p', 'forge'
    )
}
else {
    $testArgs = @('test', '--workspace')
    if ($Serial) { $testArgs += '--', '--test-threads=1' }
    Invoke-Cargo 'tests' $testArgs
}

Write-Host ""
Write-Host 'gate passed' -ForegroundColor Green