Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$release = Join-Path $root ".github\workflows\release.yml"
$readme = Join-Path $root "README.md"
$manifest = Join-Path $root "app.manifest"
$devManifest = Join-Path $root "app.dev.manifest"

if (-not (Test-Path $release)) {
    throw "Missing release workflow: $release"
}
if (-not (Test-Path $readme)) {
    throw "Missing README: $readme"
}
if (-not (Test-Path $manifest)) {
    throw "Missing release manifest: $manifest"
}
if (-not (Test-Path $devManifest)) {
    throw "Missing dev manifest: $devManifest"
}

$releaseText = Get-Content -LiteralPath $release -Raw
$readmeText = Get-Content -LiteralPath $readme -Raw
$manifestText = Get-Content -LiteralPath $manifest -Raw
$devManifestText = Get-Content -LiteralPath $devManifest -Raw

function Assert-Contains($Text, $Pattern, $Message) {
    if ($Text -notmatch $Pattern) {
        throw $Message
    }
}

function Assert-CommonControlsV6($Text, $Name) {
    Assert-Contains $Text 'Microsoft\.Windows\.Common-Controls' "$Name is missing Common Controls v6 dependency"
    Assert-Contains $Text 'version\s*=\s*"6\.0\.0\.0"' "$Name Common Controls dependency is not v6"
    Assert-Contains $Text 'publicKeyToken\s*=\s*"6595b64144ccf1df"' "$Name Common Controls dependency is missing publicKeyToken"
}

Assert-CommonControlsV6 $manifestText 'app.manifest'
Assert-CommonControlsV6 $devManifestText 'app.dev.manifest'

Assert-Contains $releaseText 'ref:\s*\$\{\{\s*github\.event_name\s*==\s*''workflow_dispatch''\s*&&\s*inputs\.tag\s*\|\|\s*github\.ref\s*\}\}' "workflow_dispatch tag input is not used by checkout"
Assert-Contains $releaseText 'Validate release tag matches Cargo\.toml version' "missing release tag/Cargo.toml version validation step"
Assert-Contains $releaseText '\$expectedTag\s*=\s*"v\$cargoVersion"' "missing expected tag derived from Cargo.toml package.version"
Assert-Contains $releaseText 'RELEASE_TAG=' "release tag is not exported for later steps"
Assert-Contains $releaseText 'cargo fmt --all -- --check' "release workflow does not run cargo fmt"
Assert-Contains $releaseText 'cargo clippy --all-targets --no-deps --locked -- -D warnings' "release workflow does not run cargo clippy with locked dependencies"
Assert-Contains $releaseText 'cargo test --all-targets' "release workflow does not run cargo test --all-targets"
Assert-Contains $releaseText 'npm ci' "release workflow does not install frontend dependencies"
Assert-Contains $releaseText 'npm audit' "release workflow does not run npm audit"
Assert-Contains $releaseText 'npm run check:design-parity' "release workflow does not run design parity check"
Assert-Contains $releaseText 'npm run build' "release workflow does not build frontend assets"
Assert-Contains $releaseText 'tag_name:\s*\$\{\{\s*env\.RELEASE_TAG\s*\}\}' "GitHub Release does not use validated release tag"

if ($readmeText -match '\[GitHub Releases\]\(#\)') {
    throw "README still contains GitHub Releases placeholder link"
}
Assert-Contains $readmeText 'Get-FileHash\s+\.\\bdo-optimizer-launcher\.exe\s+-Algorithm\s+SHA256' "README is missing SHA256 verification command"
Assert-Contains $readmeText 'Get-Content\s+\.\\SHA256SUMS\.txt' "README is missing SHA256SUMS comparison command"
Assert-Contains $readmeText 'cargo test --all-targets' "README developer test command is stale"
Assert-Contains $readmeText 'npm run check:design-parity' "README is missing design parity verification command"

Write-Host "release workflow checks passed"
