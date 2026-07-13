param(
    [Parameter(Mandatory = $true)]
    [string]$Executable,
    [Parameter(Mandatory = $true)]
    [string]$ExpectedVersion,
    [ValidateSet("requireAdministrator", "asInvoker")]
    [string]$ExpectedExecutionLevel = "requireAdministrator"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$exe = if ([System.IO.Path]::IsPathRooted($Executable)) { $Executable } else { Join-Path $root $Executable }
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) {
    throw "Tauri executable not found: $exe"
}

$distAssets = Join-Path $root "dist\assets"
$entries = @(Get-ChildItem -LiteralPath $distAssets -Filter "index-*.js" -File)
if ($entries.Count -ne 1) {
    throw "Expected exactly one Vite entry asset under $distAssets, found $($entries.Count)"
}
$entry = $entries[0]

$productVersion = (Get-Item -LiteralPath $exe).VersionInfo.ProductVersion
if ($productVersion -ne $ExpectedVersion) {
    throw "Executable ProductVersion '$productVersion' != '$ExpectedVersion'"
}

$rg = Get-Command rg -ErrorAction Stop
& $rg.Source -a -F --quiet -- $entry.Name $exe
if ($LASTEXITCODE -ne 0) {
    throw "Production frontend asset '$($entry.Name)' is not embedded in $exe"
}
& $rg.Source -a -F --quiet -- "tauri.localhost" $exe
if ($LASTEXITCODE -ne 0) {
    throw "Tauri custom protocol marker is missing from $exe"
}
& $rg.Source -a -F --quiet -- "level=`"$ExpectedExecutionLevel`"" $exe
if ($LASTEXITCODE -ne 0) {
    throw "Expected execution level '$ExpectedExecutionLevel' is missing from $exe"
}

Write-Host "Tauri production embed checks passed: $exe"
