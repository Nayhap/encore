# Installs the NSIS installer silently, runs `encore --self-test` from the
# installed copy (the way users run it), then uninstalls.
#   scripts/self-test-bundle.ps1 -Installer path\to\Encore_x.y.z_x64-setup.exe [-Video id] [-Strict]
param(
  [Parameter(Mandatory = $true)][string]$Installer,
  [string]$Video = "4D7u5KF7SP8",
  [switch]$Strict
)
$ErrorActionPreference = "Stop"
$p = Start-Process -FilePath $Installer -ArgumentList "/S" -Wait -PassThru
if ($p.ExitCode -ne 0) { throw "installer exited with $($p.ExitCode)" }
$exe = Join-Path $env:LOCALAPPDATA "Encore\encore.exe"
if (-not (Test-Path $exe)) { throw "Encore was not installed at $exe" }
$out = Join-Path $env:TEMP "encore-self-test.json"
Remove-Item $out -ErrorAction SilentlyContinue
$a = @("--self-test", $Video, "--out", "`"$out`"")
if ($Strict) { $a += "--strict" }
# Release builds are GUI programs with no console: read the report file.
$run = Start-Process -FilePath $exe -ArgumentList $a -Wait -PassThru
if (Test-Path $out) { Get-Content $out } else { Write-Host "no report written" }
$code = $run.ExitCode
$un = Join-Path $env:LOCALAPPDATA "Encore\uninstall.exe"
if (Test-Path $un) { Start-Process -FilePath $un -ArgumentList "/S" -Wait | Out-Null }
if ($code -ne 0) { throw "self-test failed with exit code $code" }
