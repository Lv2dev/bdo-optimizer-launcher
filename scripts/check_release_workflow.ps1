Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$release = Join-Path $root ".github\workflows\release.yml"
$readme = Join-Path $root "README.md"

if (-not (Test-Path $release)) {
    throw "Missing release workflow: $release"
}
if (-not (Test-Path $readme)) {
    throw "Missing README: $readme"
}

$releaseText = Get-Content -LiteralPath $release -Raw
$readmeText = Get-Content -LiteralPath $readme -Raw

function Assert-Contains($Text, $Pattern, $Message) {
    if ($Text -notmatch $Pattern) {
        throw $Message
    }
}

Assert-Contains $releaseText 'ref:\s*\$\{\{\s*github\.event_name\s*==\s*''workflow_dispatch''\s*&&\s*inputs\.tag\s*\|\|\s*github\.ref\s*\}\}' "workflow_dispatch tag input is not used by checkout"
Assert-Contains $releaseText 'Validate release tag matches Cargo\.toml version' "missing release tag/Cargo.toml version validation step"
Assert-Contains $releaseText '\$expectedTag\s*=\s*"v\$cargoVersion"' "missing expected tag derived from Cargo.toml package.version"
Assert-Contains $releaseText 'RELEASE_TAG=' "release tag is not exported for later steps"
Assert-Contains $releaseText 'cargo fmt --all -- --check' "release workflow does not run cargo fmt"
Assert-Contains $releaseText 'cargo clippy --all-targets --no-deps -- -D warnings' "release workflow does not run cargo clippy"
Assert-Contains $releaseText 'cargo test --all-targets' "release workflow does not run cargo test --all-targets"
Assert-Contains $releaseText 'scripts\\check_button_layout\.ps1' "release workflow does not run UI structure check"
Assert-Contains $releaseText 'tag_name:\s*\$\{\{\s*env\.RELEASE_TAG\s*\}\}' "GitHub Release does not use validated release tag"

if ($readmeText -match '\[GitHub Releases\]\(#\)') {
    throw "README still contains GitHub Releases placeholder link"
}
Assert-Contains $readmeText 'Get-FileHash\s+\.\\bdo-optimizer-launcher\.exe\s+-Algorithm\s+SHA256' "README is missing SHA256 verification command"
Assert-Contains $readmeText 'Get-Content\s+\.\\SHA256SUMS\.txt' "README is missing SHA256SUMS comparison command"
Assert-Contains $readmeText 'cargo test --all-targets' "README developer test command is stale"

Write-Host "release workflow checks passed"
