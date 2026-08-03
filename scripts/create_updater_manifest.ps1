param(
    [Parameter(Mandatory = $true)][string]$Version,
    [Parameter(Mandatory = $true)][string]$Tag,
    [Parameter(Mandatory = $true)][string]$Repository,
    [Parameter(Mandatory = $true)][string]$InstallerPath,
    [Parameter(Mandatory = $true)][string]$SignaturePath,
    [Parameter(Mandatory = $true)][string]$NotesPath,
    [Parameter(Mandatory = $true)][string]$OutputPath
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if ($Repository -cne "Lv2dev/bdo-optimizer-launcher") {
    throw "Updater repository is not the approved production repository: $Repository"
}
try {
    $semver = [System.Management.Automation.SemanticVersion]::new($Version)
}
catch {
    throw "Updater version is not valid SemVer: $Version"
}
if ($semver.ToString() -cne $Version -or $Tag -cne "v$Version") {
    throw "Updater tag/version mismatch: $Tag / $Version"
}

$installer = Get-Item -LiteralPath $InstallerPath -ErrorAction Stop
$signatureFile = Get-Item -LiteralPath $SignaturePath -ErrorAction Stop
$notesFile = Get-Item -LiteralPath $NotesPath -ErrorAction Stop
if ($installer.Name -cne "bdo-optimizer-launcher-setup.exe") {
    throw "Updater installer must use the canonical asset name."
}
if ($signatureFile.Name -cne "bdo-optimizer-launcher-setup.exe.sig") {
    throw "Updater signature must use the canonical asset name."
}

$signature = (Get-Content -LiteralPath $signatureFile.FullName -Raw).Trim()
if ([string]::IsNullOrWhiteSpace($signature) -or $signature -notmatch '^[A-Za-z0-9+/]+={0,2}$') {
    throw "Updater signature is not a single base64 value."
}
$notes = [IO.File]::ReadAllText($notesFile.FullName).Replace("`r`n", "`n").Replace("`r", "`n").Trim()
if ([string]::IsNullOrWhiteSpace($notes)) {
    throw "Updater release notes are empty."
}

$url = "https://github.com/$Repository/releases/download/$Tag/$($installer.Name)"
$manifest = [ordered]@{
    version = $Version
    notes = $notes
    platforms = [ordered]@{
        "windows-x86_64" = [ordered]@{
            signature = $signature
            url = $url
        }
    }
}
$json = $manifest | ConvertTo-Json -Depth 5
$outputDirectory = Split-Path -Parent $OutputPath
if ($outputDirectory) {
    New-Item -ItemType Directory -Path $outputDirectory -Force | Out-Null
}
[IO.File]::WriteAllText($OutputPath, "$json`n", [Text.UTF8Encoding]::new($false))

$roundTrip = Get-Content -LiteralPath $OutputPath -Raw | ConvertFrom-Json
$platform = $roundTrip.platforms.'windows-x86_64'
if (
    [string]$roundTrip.version -cne $Version -or
    [string]$roundTrip.notes -cne $notes -or
    [string]$platform.url -cne $url -or
    [string]$platform.signature -cne $signature
) {
    throw "Generated updater manifest failed round-trip validation."
}

Write-Host "updater manifest created: $OutputPath"
