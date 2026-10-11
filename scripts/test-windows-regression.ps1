#requires -Version 5.1
<#
Runs local regression checks. E2E uses a real debug Tauri EXE and loopback fixture.
See docs/platforms/windows-regression.md for dependencies and test boundaries.
#>
[CmdletBinding()]
param(
    [ValidateSet('All', 'Unit', 'E2E')][string]$Suite = 'All',
    [switch]$Prepare,
    [string]$PythonPath,
    [string]$EdgeDriverPath
)

$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { throw 'This regression entry requires Windows.' }
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$artifactRoot = Join-Path $repoRoot 'target/windows-regression'
# Do not follow an existing directory junction out of the repository.
foreach ($directory in @((Join-Path $repoRoot 'target'), $artifactRoot)) {
    if ((Test-Path -LiteralPath $directory) -and
        ((Get-Item -LiteralPath $directory -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "Regression artifact directory must not be a reparse point: $directory"
    }
}
$runId = 'checks-' + [DateTime]::UtcNow.ToString('yyyyMMddTHHmmss') + '-' + [Guid]::NewGuid().ToString('N').Substring(0, 8)
$checkRoot = Join-Path $artifactRoot $runId
$null = New-Item -ItemType Directory -Path $checkRoot
$steps = New-Object 'System.Collections.Generic.List[object]'
$result = [ordered]@{ suite = $Suite; passed = $false; steps = $steps; artifactDirectory = $checkRoot }

function Invoke-RegressionCheck {
    param([string]$Name, [string]$File, [string[]]$Arguments)
    $log = Join-Path $checkRoot ($Name + '.log')
    Write-Host "[$Name] running"
    $started = [DateTime]::UtcNow
    $code = -1
    $command = @(Get-Command $File -CommandType Application -ErrorAction Stop)[0]
    $previousPreference = $ErrorActionPreference
    try {
        # Windows PowerShell 5 treats native stderr as error records even when
        # the command succeeds (cargo writes progress there). Judge its exit code.
        $ErrorActionPreference = 'Continue'
        & $command.Path @Arguments *> $log
        $code = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousPreference
        $steps.Add([ordered]@{
            name = $Name; exitCode = $code; log = $log
            durationSeconds = [Math]::Round(([DateTime]::UtcNow - $started).TotalSeconds, 2)
        })
    }
    if ($code -ne 0) { throw "$Name failed (exit $code). See $log" }
    Write-Host "[$Name] PASS"
}

Push-Location $repoRoot
try {
    $null = Get-Command npm.cmd -ErrorAction Stop
    $null = Get-Command cargo -ErrorAction Stop
    if ($Suite -ne 'Unit') {
        if (Get-Process -Name 'art-term-gui' -ErrorAction SilentlyContinue) {
            throw 'Close your existing art-term window before running native E2E; this script will not stop it.'
        }
        if (-not $EdgeDriverPath) { $EdgeDriverPath = Join-Path $repoRoot 'target/gui-tools/edge/msedgedriver.exe' }
        $EdgeDriverPath = (Resolve-Path -LiteralPath $EdgeDriverPath -ErrorAction Stop).Path
        if (-not $PythonPath) { $PythonPath = Join-Path $artifactRoot 'venv/Scripts/python.exe' }
    }
    if ($Prepare) {
        Invoke-RegressionCheck 'npm-ci' 'npm.cmd' @('--prefix', 'apps/desktop', 'ci')
        if ($Suite -ne 'Unit') {
            if (-not (Test-Path -LiteralPath $PythonPath -PathType Leaf)) {
                $defaultPython = Join-Path $artifactRoot 'venv/Scripts/python.exe'
                if ([IO.Path]::GetFullPath($PythonPath) -ne [IO.Path]::GetFullPath($defaultPython)) {
                    throw 'A custom PythonPath must already exist; Prepare only creates the repository test venv.'
                }
                Invoke-RegressionCheck 'python-venv' 'python' @('-m', 'venv', (Join-Path $artifactRoot 'venv'))
            }
            Invoke-RegressionCheck 'python-dependencies' $PythonPath @('-m', 'pip', 'install', '-r', 'scripts/windows-regression/requirements.txt')
        }
    }
    if ($Suite -ne 'E2E') {
        Invoke-RegressionCheck 'frontend-typecheck' 'npm.cmd' @('--prefix', 'apps/desktop', 'run', 'typecheck')
        Invoke-RegressionCheck 'frontend-tests' 'npm.cmd' @('--prefix', 'apps/desktop', 'test')
        Invoke-RegressionCheck 'rust-format' 'cargo' @('fmt', '--all', '--check')
        Invoke-RegressionCheck 'rust-tests' 'cargo' @('test', '--workspace', '--locked')
        Invoke-RegressionCheck 'gui-check' 'cargo' @('check', '-p', 'art-term-gui', '--locked')
    }
    if ($Suite -ne 'Unit') {
        $PythonPath = (Resolve-Path -LiteralPath $PythonPath -ErrorAction Stop).Path
        Invoke-RegressionCheck 'python-preflight' $PythonPath @('-c', 'import aiohttp, cryptography, requests, selenium, PIL')
        Invoke-RegressionCheck 'frontend-build' 'npm.cmd' @('--prefix', 'apps/desktop', 'run', 'build')
        # Debug only: the existing release compile guard forbids openab-fixture.
        Invoke-RegressionCheck 'fixture-build' 'cargo' @('build', '-p', 'art-term-gui', '--locked', '--features', 'openab-fixture')
        Invoke-RegressionCheck 'native-e2e' $PythonPath @(
            'scripts/windows-regression/run.py', '--exe', (Join-Path $repoRoot 'target/debug/art-term-gui.exe'),
            '--driver', $EdgeDriverPath, '--output-root', $artifactRoot
        )
    }
    $result.passed = $true
} catch {
    $result.failure = $_.Exception.Message
    Write-Warning $_.Exception.Message
} finally {
    $result.steps = @($steps.ToArray())
    $result | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $checkRoot 'result.json') -Encoding UTF8
    Pop-Location
}
Write-Host "Result: $(Join-Path $checkRoot 'result.json')"
if (-not $result.passed) { exit 1 }
exit 0
