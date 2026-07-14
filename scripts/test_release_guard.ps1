Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
. (Join-Path $root "scripts\installer_acl_guard.ps1")

$packageReadOnlySddl = "O:BAD:(A;;0x1200a9;;;S-1-15-2-1)(A;;0x1200a9;;;S-1-15-2-2)"
if ((Get-SddlOwnerSid $packageReadOnlySddl) -ne "S-1-5-32-544") {
    throw "Raw SDDL owner SID was not preserved"
}
if (Test-SddlDangerousUntrustedAce $packageReadOnlySddl @("S-1-5-32-545")) {
    throw "Read-only app package SIDs were treated as writable"
}
if (-not (Test-SddlDangerousUntrustedAce "O:BAD:(A;;0x1301bf;;;BU)" @("S-1-5-32-545"))) {
    throw "Writable Users SID was not rejected"
}
if (-not (Test-SddlDangerousUntrustedAce "O:BA" @("S-1-5-32-545"))) {
    throw "Null DACL was not rejected"
}
if (Test-SddlDangerousUntrustedAce "O:BAD:(A;IO;GA;;;BU)" @("S-1-5-32-545")) {
    throw "Inherit-only ACE was incorrectly applied to the current path"
}

$fixture = Join-Path $env:TEMP ("bdo-release-guard-" + [Guid]::NewGuid().ToString("N"))
$files = @(
    ".gitattributes",
    ".github\workflows\release.yml",
    ".github\workflows\ci.yml",
    "scripts\check_release_workflow.ps1",
    "scripts\installer_acl_guard.ps1",
    "scripts\smoke_test_installer.ps1",
    "README.md",
    "app.manifest",
    "app.dev.manifest",
    "Cargo.toml",
    "Cargo.lock",
    "build.rs",
    "src\main.rs",
    "package.json",
    "package-lock.json",
    "tauri.conf.json",
    "windows\installer.nsi",
    "windows\hooks.nsh",
    "web\src\browserPreview.js",
    "docs\distribution\manual.html",
    "docs\distribution\manual.build.json",
    "docs\distribution\manual.pdf"
)
$files += @(
    Get-ChildItem -LiteralPath (Join-Path $root "docs\distribution\screenshots") -File -Recurse |
        ForEach-Object { [IO.Path]::GetRelativePath($root, $_.FullName) }
)

$package = Get-Content -LiteralPath (Join-Path $root "package.json") -Raw | ConvertFrom-Json
$expectedTag = "v$($package.version)"

function Copy-Fixture {
    if (Test-Path -LiteralPath $fixture) { Remove-Item -LiteralPath $fixture -Recurse -Force }
    foreach ($relative in $files) {
        $destination = Join-Path $fixture $relative
        New-Item -ItemType Directory -Path (Split-Path -Parent $destination) -Force | Out-Null
        Copy-Item -LiteralPath (Join-Path $root $relative) -Destination $destination
    }
}

function Invoke-Guard {
    & pwsh -NoProfile -File (Join-Path $fixture "scripts\check_release_workflow.ps1") -ReleaseTag $expectedTag *> $null
    return $LASTEXITCODE
}

function Invoke-GuardWithTag([string]$tag) {
    & pwsh -NoProfile -File (Join-Path $fixture "scripts\check_release_workflow.ps1") -ReleaseTag $tag *> $null
    return $LASTEXITCODE
}

function Assert-MutationRejected([string]$relative, [string]$pattern, [string]$replacement) {
    Copy-Fixture
    $path = Join-Path $fixture $relative
    $text = Get-Content -LiteralPath $path -Raw
    $changed = $text -replace $pattern, $replacement
    if ($changed -eq $text) { throw "Fixture mutation did not change $relative" }
    [System.IO.File]::WriteAllText($path, $changed, [System.Text.UTF8Encoding]::new($false))
    if ((Invoke-Guard) -eq 0) { throw "Release guard accepted invalid mutation: $relative" }
}

function Assert-BinaryMutationRejected([string]$relative) {
    Copy-Fixture
    $path = Join-Path $fixture $relative
    $bytes = [IO.File]::ReadAllBytes($path)
    if ($bytes.Length -lt 101) { throw "Fixture is too small for binary mutation: $relative" }
    $bytes[100] = $bytes[100] -bxor 1
    [IO.File]::WriteAllBytes($path, $bytes)
    if ((Invoke-Guard) -eq 0) { throw "Release guard accepted stale manual input: $relative" }
}

function Assert-RipgrepInstallAfterEmbedRejected {
    Copy-Fixture
    $path = Join-Path $fixture ".github\workflows\release.yml"
    $text = Get-Content -LiteralPath $path -Raw
    $installPattern = '(?ms)^\s{6}- name: Install ripgrep\r?\n\s{8}run: cargo install ripgrep --version 15\.1\.0 --locked\r?\n'
    $installMatch = [regex]::Match($text, $installPattern)
    if (-not $installMatch.Success) { throw "Ripgrep install step was not found in fixture" }
    $withoutInstall = $text.Remove($installMatch.Index, $installMatch.Length)
    $smokeStep = [regex]::Match($withoutInstall, '(?m)^\s{6}- name: Smoke test install, upgrade, and uninstall cleanup\s*$')
    if (-not $smokeStep.Success) { throw "Installer smoke step was not found in fixture" }
    $newline = if ($text.Contains("`r`n")) { "`r`n" } else { "`n" }
    $changed = $withoutInstall.Insert($smokeStep.Index, $installMatch.Value + $newline)
    [System.IO.File]::WriteAllText($path, $changed, [System.Text.UTF8Encoding]::new($false))
    if ((Invoke-Guard) -eq 0) { throw "Release guard accepted ripgrep installation after the embed check" }
}

try {
    Copy-Fixture
    if ((Invoke-Guard) -ne 0) { throw "Valid release fixture was rejected" }
    Assert-MutationRejected "app.manifest" 'level="requireAdministrator"' 'level="asInvoker"'
    Assert-MutationRejected "app.dev.manifest" 'level="asInvoker"' 'level="requireAdministrator"'
    Assert-MutationRejected "package.json" '"tauri:build":\s*"tauri build"' '"tauri:build": "cargo build --release"'
    Assert-MutationRejected ".gitattributes" '(?m)^docs/distribution/manual\.html text eol=lf\r?$' 'docs/distribution/manual.html text eol=crlf'
    Assert-MutationRejected ".gitattributes" '(?m)^docs/distribution/manual\.html text eol=lf\r?$' "docs/distribution/manual.html text eol=lf`r`ndocs/distribution/manual.html text eol=crlf"
    Assert-MutationRejected ".gitattributes" '(?m)^windows/hooks\.nsh text eol=lf\r?$' 'windows/hooks.nsh text eol=crlf'
    Assert-MutationRejected ".gitattributes" '(?m)^windows/installer\.nsi text eol=lf\r?$' 'windows/installer.nsi text eol=crlf'
    Assert-MutationRejected "package.json" '"@tauri-apps/cli":\s*"2\.11\.2"' '"@tauri-apps/cli": "^2.0.0"'
    Assert-MutationRejected "tauri.conf.json" ('"version":\s*"' + [regex]::Escape([string]$package.version) + '"') '"version": "9.9.9"'
    Assert-MutationRejected "tauri.conf.json" '"template":\s*"\./windows/installer\.nsi"' '"template": "./windows/other.nsi"'
    Assert-MutationRejected "windows\installer.nsi" 'existing NSIS uninstaller is never elevated' 'existing NSIS uninstaller may be elevated'
    Assert-MutationRejected "windows\installer.nsi" 'Unicode true' 'Unicode false'
    Assert-MutationRejected "windows\hooks.nsh" '/reset /T /L /Q' '/reset /T /Q'
    Assert-MutationRejected "windows\hooks.nsh" '\.r4' '.rR4'
    Assert-MutationRejected "windows\hooks.nsh" 'NSIS_HOOK_PREDELETE' 'NSIS_HOOK_POSTUNINSTALL'
    Assert-MutationRejected "windows\hooks.nsh" '!macro NSIS_HOOK_PREINSTALL' "!macro NSIS_HOOK_PREINSTALL`r`n  ExecWait '`"`$SYSDIR\cmd.exe`" /c whoami' `$R9"
    Assert-MutationRejected "windows\hooks.nsh" '/Query /TN "\$\{TASK_NAME\}"' '/Run /TN "${TASK_NAME}"'
    Assert-MutationRejected "windows\hooks.nsh" '\$WINDIR\\System32\\Tasks\\\$\{TASK_NAME\}' '$WINDIR\System32\Missing\${TASK_NAME}'
    Assert-MutationRejected "windows\hooks.nsh" '\$\{DisableX64FSRedirection\}' '${EnableX64FSRedirection}'
    Assert-MutationRejected "scripts\installer_acl_guard.ps1" 'RawSecurityDescriptor' 'CommonSecurityDescriptor'
    Assert-MutationRejected ".github\workflows\release.yml" 'cargo-audit --version 0\.22\.2 --locked' 'cargo-audit --version 0.22.1 --locked'
    Assert-MutationRejected ".github\workflows\release.yml" '(?m)^(\s*)run:\s*cargo audit\s*$' '$1run: cargo test'
    Assert-MutationRejected ".github\workflows\release.yml" '(?m)^(\s*)run:\s*cargo install ripgrep --version 15\.1\.0 --locked\s*$' '$1run: cargo --version'
    Assert-MutationRejected ".github\workflows\release.yml" 'ripgrep --version 15\.1\.0 --locked' 'ripgrep --version 15.0.0 --locked'
    Assert-MutationRejected ".github\workflows\release.yml" '(?m)^(\s*)run:\s*cargo install ripgrep --version 15\.1\.0 --locked\s*$' ('${1}run: cargo install ripgrep --version 15.1.0 --locked' + "`r`n" + '${1}run: cargo install ripgrep --version 15.1.0 --locked')
    Assert-RipgrepInstallAfterEmbedRejected
    Assert-MutationRejected ".github\workflows\release.yml" '(?m)^  workflow_dispatch:' '  push:'
    Assert-MutationRejected ".github\workflows\release.yml" '(?m)^(\s+)actions:\s+read\s*$' '$1actions: none'
    Assert-MutationRejected ".github\workflows\release.yml" '\$env:WORKFLOW_REF -cne "refs/heads/\$env:DEFAULT_BRANCH"' '$env:WORKFLOW_REF -ceq "refs/heads/$env:DEFAULT_BRANCH"'
    Assert-MutationRejected ".github\workflows\release.yml" 'compare/\$tagCommit\.\.\.\$env:DEFAULT_BRANCH' 'compare/$env:DEFAULT_BRANCH...$tagCommit'
    Assert-MutationRejected ".github\workflows\release.yml" 'actions/workflows/ci\.yml/runs\?head_sha=\$tagCommit&status=success' 'actions/workflows/ci.yml/runs?status=success'
    Assert-MutationRejected ".github\workflows\release.yml" '(?m)^(\s+)tag_commit:\s*\$\{\{ steps\.release_meta\.outputs\.tag_commit \}\}\s*$' '$1tag_commit: missing'
    Assert-MutationRejected ".github\workflows\release.yml" '\$target\.sha -cne \$env:EXPECTED_COMMIT' '$target.sha -ceq $env:EXPECTED_COMMIT'
    Assert-MutationRejected ".github\workflows\release.yml" 'compare/\$env:EXPECTED_COMMIT\.\.\.\$env:DEFAULT_BRANCH' 'compare/$env:DEFAULT_BRANCH...$env:EXPECTED_COMMIT'
    Assert-MutationRejected ".github\workflows\release.yml" '(?m)^(\s+)environment:\s*release\s*$' '$1environment: unprotected'
    Assert-MutationRejected ".github\workflows\release.yml" '(?m)^(\s+)\$tag = \$env:RELEASE_TAG\s*$' '$1$tag = "${{ inputs.tag }}"'
    foreach ($maliciousTag in @('v0.2.0$(Write-Error injected)', 'v0.2.0";Write-Error injected;"', 'v0.2.0;Write-Error injected')) {
        Copy-Fixture
        if ((Invoke-GuardWithTag $maliciousTag) -eq 0) { throw "Release guard accepted an injectable tag: $maliciousTag" }
    }
    Assert-MutationRejected "docs\distribution\manual.html" ('버전:\s*v' + [regex]::Escape([string]$package.version)) '버전: v9.9.9'
    Assert-MutationRejected "docs\distribution\manual.html" '<title>' '<title>stale '
    Assert-BinaryMutationRejected "docs\distribution\screenshots\01-control.png"
    Copy-Fixture
    Copy-Item -LiteralPath (Join-Path $fixture "docs\distribution\screenshots\01-control.png") -Destination (Join-Path $fixture "docs\distribution\screenshots\99-extra.png")
    if ((Invoke-Guard) -eq 0) { throw "Release guard accepted an unrecorded manual input" }
    Write-Host "release guard behavioral tests passed"
    $global:LASTEXITCODE = 0
} finally {
    if (Test-Path -LiteralPath $fixture) { Remove-Item -LiteralPath $fixture -Recurse -Force }
}
