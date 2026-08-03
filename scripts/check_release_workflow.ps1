param(
    [string]$ReleaseTag = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
function Read-RootFile([string]$relative) {
    $path = Join-Path $root $relative
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Missing file: $relative" }
    return Get-Content -LiteralPath $path -Raw
}
function Assert-Match([string]$text, [string]$pattern, [string]$message) {
    if ($text -notmatch $pattern) { throw $message }
}
function Assert-NoWorkflowContextInRun([string]$text) {
    $lines = $text -split "`r?`n"
    for ($index = 0; $index -lt $lines.Count; $index++) {
        $match = [regex]::Match($lines[$index], '^(\s*)run:\s*(.*)$')
        if (-not $match.Success) { continue }
        $indent = $match.Groups[1].Value.Length
        $inline = $match.Groups[2].Value
        if ($inline -notin @('', '|', '>', '>-', '|-') -and $inline -match '\$\{\{') {
            throw "GitHub context must be passed through env, not interpolated into run source"
        }
        for ($bodyIndex = $index + 1; $bodyIndex -lt $lines.Count; $bodyIndex++) {
            if ($lines[$bodyIndex].Trim().Length -eq 0) { continue }
            $bodyIndent = $lines[$bodyIndex].Length - $lines[$bodyIndex].TrimStart().Length
            if ($bodyIndent -le $indent) { break }
            if ($lines[$bodyIndex] -match '\$\{\{') {
                throw "GitHub context must be passed through env, not interpolated into run source"
            }
        }
    }
}
function Assert-CommonControlsV6([string]$text, [string]$name) {
    Assert-Match $text 'Microsoft\.Windows\.Common-Controls' "$name is missing Common Controls v6"
    Assert-Match $text 'version\s*=\s*"6\.0\.0\.0"' "$name Common Controls dependency is not v6"
}

$release = Read-RootFile ".github\workflows\release.yml"
$ci = Read-RootFile ".github\workflows\ci.yml"
$gitAttributes = Read-RootFile ".gitattributes"
$readme = Read-RootFile "README.md"
$manifest = Read-RootFile "app.manifest"
$devManifest = Read-RootFile "app.dev.manifest"
$cargo = Read-RootFile "Cargo.toml"
$cargoLock = Read-RootFile "Cargo.lock"
$package = (Read-RootFile "package.json") | ConvertFrom-Json
$packageLock = (Read-RootFile "package-lock.json") | ConvertFrom-Json -AsHashtable
$tauri = (Read-RootFile "tauri.conf.json") | ConvertFrom-Json
$preview = Read-RootFile "web\src\browserPreview.js"
$manual = Read-RootFile "docs\distribution\manual.html"
$manualBuild = (Read-RootFile "docs\distribution\manual.build.json") | ConvertFrom-Json -AsHashtable
$installerTemplate = Read-RootFile "windows\installer.nsi"
$installerHooks = Read-RootFile "windows\hooks.nsh"
$installerSmoke = Read-RootFile "scripts\smoke_test_installer.ps1"
$installerAclGuard = Read-RootFile "scripts\installer_acl_guard.ps1"
$updaterManifest = Read-RootFile "scripts\create_updater_manifest.ps1"
$updaterManifestTest = Read-RootFile "scripts\test_updater_manifest.ps1"

Assert-CommonControlsV6 $manifest "app.manifest"
Assert-CommonControlsV6 $devManifest "app.dev.manifest"
$normalizedGitAttributes = $gitAttributes.Replace("`r`n", "`n").Replace("`r", "`n").TrimEnd([char[]]"`n")
$expectedGitAttributes = @(
    "docs/distribution/manual.html text eol=lf"
    "windows/hooks.nsh text eol=lf"
    "windows/installer.nsi text eol=lf"
) -join "`n"
if ($normalizedGitAttributes -cne $expectedGitAttributes) {
    throw "Whole-file SHA inputs must have exact deterministic LF attributes"
}
function Assert-ExecutionLevel([string]$text, [string]$expected, [string]$name) {
    [xml]$xml = $text
    $nodes = @($xml.SelectNodes("//*[local-name()='requestedExecutionLevel']"))
    if ($nodes.Count -ne 1 -or $nodes[0].level -ne $expected) {
        throw "$name must contain exactly one requestedExecutionLevel='$expected'"
    }
}
Assert-ExecutionLevel $manifest "requireAdministrator" "app.manifest"
Assert-ExecutionLevel $devManifest "asInvoker" "app.dev.manifest"

$versionMatch = [regex]::Match($cargo, '(?m)^version\s*=\s*"([^"]+)"')
if (-not $versionMatch.Success) { throw "Cargo.toml package.version not found" }
$version = $versionMatch.Groups[1].Value
$releaseNotesRelative = "docs\releases\$version.md"
$releaseNotesPath = Join-Path $root $releaseNotesRelative
if (-not (Test-Path -LiteralPath $releaseNotesPath -PathType Leaf)) {
    throw "Korean release notes are missing: $releaseNotesRelative"
}
$releaseNotes = [IO.File]::ReadAllText($releaseNotesPath).Replace("`r`n", "`n").Replace("`r", "`n").Trim()
$releaseNoteLines = @($releaseNotes -split "`n")
if ($releaseNoteLines.Count -lt 2 -or $releaseNoteLines[0] -cne "## 변경 사항") {
    throw "Release notes must start with the exact Korean heading '## 변경 사항'"
}
$releaseBullets = @($releaseNoteLines | Select-Object -Skip 1 | Where-Object { $_.Length -gt 0 })
if ($releaseBullets.Count -eq 0 -or @($releaseBullets | Where-Object { $_ -notmatch '^- \S' }).Count -gt 0) {
    throw "Release notes may contain only non-empty Markdown bullets after the heading"
}
if (@($releaseBullets | Where-Object { $_ -notmatch '[가-힣]' }).Count -gt 0) {
    throw "Every release note bullet must contain a Korean change description"
}
if ($releaseNotes -match '(?i)(?<![A-Za-z])(PR|CI)(?![A-Za-z])|SHA-?256|SmartScreen|커밋|마일스톤|테스트|빌드|검증|요구사항') {
    throw "Release notes contain internal verification or generic installation details"
}
$lockRootVersion = $packageLock['packages']['']['version']
$versions = @{
    "package.json" = [string]$package.version
    "package-lock.json root" = [string]$packageLock['version']
    "package-lock.json packages['']" = [string]$lockRootVersion
    "tauri.conf.json" = [string]$tauri.version
}
foreach ($entry in $versions.GetEnumerator()) {
    if ($entry.Value -ne $version) { throw "$($entry.Key) version '$($entry.Value)' != Cargo version '$version'" }
}
$metadataText = & cargo metadata --locked --no-deps --format-version 1 --manifest-path (Join-Path $root "Cargo.toml")
if ($LASTEXITCODE -ne 0) { throw "cargo metadata --locked failed" }
$metadata = $metadataText | ConvertFrom-Json
$rootPackage = $metadata.packages | Where-Object { $_.name -eq "bdo-optimizer-launcher" } | Select-Object -First 1
if ($null -eq $rootPackage -or [string]$rootPackage.version -ne $version) {
    throw "Cargo.lock/root package version does not match Cargo.toml"
}
$previewVersions = [regex]::Matches($preview, 'appVersion:\s*"([^"]+)"')
if ($previewVersions.Count -eq 0) { throw "browser preview appVersion not found" }
foreach ($match in $previewVersions) {
    if ($match.Groups[1].Value -ne $version) { throw "browser preview version mismatch" }
}
Assert-Match $manual ("버전:\s*v" + [regex]::Escape($version)) "manual.html version does not match Cargo"
if ([regex]::Matches($manual, ("v" + [regex]::Escape($version))).Count -lt 2) {
    throw "manual.html cover/footer versions are not both synchronized"
}
if (-not (Test-Path -LiteralPath (Join-Path $root "docs\distribution\manual.pdf") -PathType Leaf)) {
    throw "manual.pdf is missing"
}
$manualRoot = Join-Path $root "docs\distribution"
$manualHtmlPath = Join-Path $manualRoot "manual.html"
$manualPdfPath = Join-Path $root "docs\distribution\manual.pdf"
if ([string]$manualBuild['version'] -ne $version) { throw "manual build manifest version does not match Cargo" }
$manualInputFiles = @((Get-Item -LiteralPath $manualHtmlPath)) + @(
    Get-ChildItem -LiteralPath (Join-Path $manualRoot "screenshots") -File -Recurse
)
$manualEntries = @(
    $manualInputFiles | ForEach-Object {
        [pscustomobject]@{
            Path = [IO.Path]::GetRelativePath($manualRoot, $_.FullName).Replace('\', '/')
            Hash = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash
        }
    } | Sort-Object -Property Path -CaseSensitive
)
$recordedInputs = $manualBuild['inputs']
if ($recordedInputs -isnot [Collections.IDictionary] -or $recordedInputs.Count -ne $manualEntries.Count) {
    throw "manual build input set does not match the current manual input tree"
}
foreach ($entry in $manualEntries) {
    if (-not $recordedInputs.Contains($entry.Path) -or [string]$recordedInputs[$entry.Path] -cne $entry.Hash) {
        throw "manual input changed after manual.pdf was generated: $($entry.Path)"
    }
}
$treeMaterial = ($manualEntries | ForEach-Object { "$($_.Path)`0$($_.Hash)`n" }) -join ""
$treeBytes = [Text.UTF8Encoding]::new($false).GetBytes($treeMaterial)
$treeHash = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($treeBytes))
if ([string]$manualBuild['inputTreeSha256'] -cne $treeHash) {
    throw "manual input tree hash does not match the recorded build"
}
if ([string]$manualBuild['pdfSha256'] -cne (Get-FileHash -LiteralPath $manualPdfPath -Algorithm SHA256).Hash) {
    throw "manual.pdf does not match the recorded manual build"
}

if ($ReleaseTag) {
    if (-not $ReleaseTag.StartsWith("v")) { throw "Invalid release tag: $ReleaseTag" }
    try { $semver = [System.Management.Automation.SemanticVersion]::new($ReleaseTag.Substring(1)) }
    catch { throw "Invalid SemVer release tag: $ReleaseTag" }
    if ($semver.ToString() -cne $ReleaseTag.Substring(1)) { throw "Release tag is not canonical SemVer: $ReleaseTag" }
    if ($ReleaseTag -ne "v$version") { throw "Release tag '$ReleaseTag' != version 'v$version'" }
}

if ([string]$package.scripts.'tauri:build' -ne 'tauri build') {
    throw "package.json tauri:build must be exactly 'tauri build'"
}
$requiredPostcssVersion = '8.5.25'
if ([string]$package.overrides.postcss -cne $requiredPostcssVersion) {
    throw "package.json must pin the PostCSS security override to $requiredPostcssVersion"
}
$postcssLock = $packageLock['packages']['node_modules/postcss']
if ($postcssLock -isnot [Collections.IDictionary] -or
    [string]$postcssLock['version'] -cne $requiredPostcssVersion -or
    [string]$postcssLock['resolved'] -cne "https://registry.npmjs.org/postcss/-/postcss-$requiredPostcssVersion.tgz") {
    throw "package-lock.json must resolve PostCSS exactly to $requiredPostcssVersion"
}
if (-not $tauri.bundle.active) { throw "Tauri bundle must be active" }
if ($tauri.bundle.createUpdaterArtifacts -ne $true) { throw "Tauri updater artifacts must be enabled" }
if (@($tauri.bundle.targets) -notcontains 'nsis') { throw "Tauri bundle targets must include nsis" }
if ($tauri.bundle.windows.nsis.installMode -ne 'perMachine') { throw "NSIS installMode must be perMachine" }
if ($tauri.bundle.windows.nsis.installerHooks -ne './windows/hooks.nsh') { throw "NSIS uninstall cleanup hook is missing" }
if ($tauri.bundle.windows.nsis.template -ne './windows/installer.nsi') { throw "Pinned NSIS template is missing" }
$expectedUpdaterPublicKey = 'dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEU1QUI2NTg0MjEwQTMwRTAKUldUZ01Bb2hoR1dyNVVYUFdmVmRXa3lPVDB2aVJCZ2JraGJZbnFOYklXNEp2Qk02S01XUXlwSTIK'
if ([string]$tauri.plugins.updater.pubkey -cne $expectedUpdaterPublicKey) { throw "Updater public key changed without explicit migration" }
$updaterEndpoints = @($tauri.plugins.updater.endpoints)
if ($updaterEndpoints.Count -ne 1 -or [string]$updaterEndpoints[0] -cne 'https://github.com/Lv2dev/bdo-optimizer-launcher/releases/latest/download/latest.json') {
    throw "Updater endpoint must be the exact production GitHub latest.json URL"
}
if ([string]$tauri.plugins.updater.windows.installMode -cne 'passive') { throw "Windows updater installMode must be passive" }
Assert-Match $cargo '(?m)^tauri-plugin-updater\s*=\s*"2"\s*$' "Rust updater plugin dependency is missing"
Assert-Match $cargo '(?m)^minisign-verify\s*=\s*"=0\.2\.5"\s*$' "Artifact signature verifier must be pinned"
Assert-Match $cargoLock '(?ms)^name = "tauri-plugin-updater"\r?\nversion = "2\.10\.1"\r?$' "Locked updater plugin version changed"
if ([string]$package.devDependencies.'@tauri-apps/cli' -cne '2.11.2') { throw "Tauri CLI must be pinned to 2.11.2" }
if ([string]$packageLock['packages']['node_modules/@tauri-apps/cli']['version'] -cne '2.11.2') {
    throw "package-lock Tauri CLI must be 2.11.2"
}
Assert-Match $installerTemplate 'upstream commit: 499df79be65ef8c0670abc0207cd9e37b55d8491' "NSIS template provenance is missing"
$installerTemplateHash = (Get-FileHash -LiteralPath (Join-Path $root "windows\installer.nsi") -Algorithm SHA256).Hash
if ($installerTemplateHash -cne '38C43D7D6BE9EDF5639B05FB502A7B328889577F9A9746B81F3B12C3699E9C84') {
    throw "Pinned NSIS template hash changed without explicit review"
}
$installerHooksHash = (Get-FileHash -LiteralPath (Join-Path $root "windows\hooks.nsh") -Algorithm SHA256).Hash
if ($installerHooksHash -cne 'BAC7EBBC48C502D785E8280D9606C55B9D1627BA7C617670C0CD807BDFA2B4B2') {
    throw "Pinned privileged NSIS hooks hash changed without explicit review"
}
if ($installerTemplate -match 'StrCpy\s+\$R1\s+"\$R1 /UPDATE"') { throw "NSIS upgrades must not elevate the previous uninstaller" }
Assert-Match $installerTemplate '!if "\$\{INSTALLMODE\}" != "perMachine"[\s\S]*?!insertmacro MUI_PAGE_DIRECTORY[\s\S]*?!endif' "per-machine installer must not expose the directory page"
Assert-Match $installerTemplate 'Section Install[\s\S]*?NSIS_HOOK_PREINSTALL[\s\S]*?SetOutPath \$INSTDIR' "NSIS preinstall guard must run before SetOutPath"
Assert-Match $installerTemplate 'existing NSIS uninstaller is never elevated by a newer installer' "NSIS upgrades must not execute an untrusted existing uninstaller"
Assert-Match $installerTemplate '(?ms)Function \.onInstSuccess.*?/R.*?/ARGS.*?nsis_tauri_utils::RunAsUser' "NSIS updater must restart the installed app with forwarded arguments"
Assert-Match $installerTemplate '\$\{If\} \$\{Silent\}[\s\S]*?\$WixMode <> 1[\s\S]*?Goto reinst_done' "silent NSIS upgrades must use the in-place path"
Assert-Match $installerTemplate '\$PassiveMode = 1[\s\S]*?\$WixMode <> 1[\s\S]*?Goto reinst_done' "passive NSIS upgrades must use the in-place path"
Assert-Match $installerTemplate '\$WixMode = 1[\s\S]*?ReadRegStr \$R1 HKLM "\$R6" "UninstallString"[\s\S]*?ExecWait' "WiX migration must retain its trusted msiexec uninstall path"
if ($installerHooks -match '_\?=') { throw "NSIS hooks must not infer upgrade mode from _?=" }
Assert-Match $installerHooks '!macro NSIS_HOOK_PREINSTALL' "NSIS preinstall ACL hook is missing"
Assert-Match $installerHooks '!macro NSIS_HOOK_POSTINSTALL' "NSIS postinstall ACL hook is missing"
Assert-Match $installerHooks '!macro NSIS_HOOK_PREDELETE' "NSIS pre-delete task cleanup hook is missing"
Assert-Match $installerHooks '!macro BDO_DELETE_TASK_FAIL_CLOSED TASK_NAME' "NSIS task cleanup must use fail-closed deletion"
Assert-Match $installerHooks '!macro BDO_TASK_FILE_EXISTS TASK_NAME RESULT_VAR' "NSIS task cleanup must distinguish absent tasks from query failures"
Assert-Match $installerHooks '\$\{DisableX64FSRedirection\}[\s\S]*?\$WINDIR\\System32\\Tasks\\\$\{TASK_NAME\}[\s\S]*?\$\{EnableX64FSRedirection\}' "NSIS task cleanup must inspect the protected root task file without WOW64 redirection"
Assert-Match $installerHooks '/Query /TN "\$\{TASK_NAME\}"[\s\S]*?/Delete /TN "\$\{TASK_NAME\}" /F[\s\S]*?/Query /TN "\$\{TASK_NAME\}"' "NSIS task cleanup must verify absence after deletion"
Assert-Match $installerHooks 'Abort "BDO Optimizer 예약 작업 삭제에 실패했습니다: \$\{TASK_NAME\}"' "NSIS task cleanup must fail closed when a task remains"
Assert-Match $installerHooks 'Abort "BDO Optimizer 예약 작업 조회에 실패했습니다: \$\{TASK_NAME\}"' "NSIS task cleanup must fail closed when query fails but a task file remains"
foreach ($taskName in @('BDO_Optimizer_Launcher_Autostart', 'BDO_Auto_Shutdown_Once', 'BDO_Auto_Shutdown_Weekly')) {
    Assert-Match $installerHooks ('BDO_DELETE_TASK_FAIL_CLOSED "' + [regex]::Escape($taskName) + '"') "NSIS cleanup is missing task: $taskName"
}
if ($installerHooks -match '!macro NSIS_HOOK_PREUNINSTALL') { throw "Task cleanup must not run before uninstall is committed" }
Assert-Match $installerTemplate 'Section Uninstall[\s\S]*?CheckIfAppIsRunning[\s\S]*?NSIS_HOOK_PREDELETE[\s\S]*?Delete "\$INSTDIR\\\$\{MAINBINARYNAME\}\.exe"' "Task cleanup must run after cancellation checks and before installed files are deleted"
Assert-Match $installerHooks '\$PROGRAMFILES64\\\$\{PRODUCTNAME\}' "NSIS hook must enforce the Program Files x64 path"
Assert-Match $installerHooks 'FILE_ATTRIBUTE_REPARSE_POINT' "NSIS hook must reject reparse install directories"
Assert-Match $installerHooks 'Function BDO_RejectReparseTree' "NSIS hook must reject descendant reparse points"
if ($installerHooks -match '\.rR[0-9]') { throw "NSIS System::Call outputs must use valid numeric registers" }
if ([regex]::Matches($installerHooks, '/T /L /Q').Count -ne 3) { throw "All recursive icacls operations must use no-follow /L" }
foreach ($argument in @('/setowner', '/reset', '/verify')) {
    Assert-Match $installerHooks ([regex]::Escape($argument)) "NSIS ACL hardening is missing $argument"
}
if ($installerSmoke -match '_\?=') { throw "Installer smoke must exercise the real setup upgrade path" }
Assert-Match $installerSmoke 'DisplayVersion[^\r\n]+0\.1\.9' "Installer smoke must force a version replacement"
Assert-Match $installerSmoke '/grant[^\r\n]+\(OI\)\(CI\)M' "Installer smoke must seed a user-writable ACL"
Assert-Match $installerSmoke 'installer_acl_guard\.ps1' "Installer smoke must load the raw SID ACL guard"
Assert-Match $installerAclGuard 'RawSecurityDescriptor' "Installer ACL guard must inspect raw SDDL"
Assert-Match $installerAclGuard 'GetSecurityDescriptorSddlForm' "Installer ACL guard must read owner and ACE SIDs without name translation"
Assert-Match $installerSmoke 'taskXmlBefore' "Installer smoke must compare task XML across upgrade"
Assert-Match $installerSmoke 'Invoke-Checked \$installerPath @\("/P", "/UPDATE", "/R", "/ARGS", "--minimized"\)' "Installer smoke must exercise the updater passive install and restart arguments"
Assert-Match $installerSmoke 'Wait-InstalledProcess \$true 15' "Installer smoke must verify updater restart"
Assert-Match $installerSmoke 'Stop-Process -Id \$process\.ProcessId -Force' "Installer smoke must stop only the exact restarted installed process"
Assert-Match $installerSmoke 'First install unexpectedly created task' "Installer smoke must assert initial task absence before seeding fixtures"
Assert-Match $installerSmoke 'Untrusted existing uninstaller was executed' "Installer smoke must prove upgrades do not execute the previous UninstallString"
Assert-Match $installerSmoke 'already-absent uninstall task case' "Installer smoke must verify that an already-absent task is accepted"
Assert-Match $installerSmoke 'Uninstall left orphan task' "Installer smoke must reject orphan tasks after uninstall"
Assert-Match $installerSmoke '(?ms)\}\s*finally\s*\{.*?\}\s*Write-Host "installer install/upgrade/uninstall smoke test passed"\s*\r?\n\$global:LASTEXITCODE = 0\s*$' "Installer smoke must report success and normalize the native exit code only after cleanup"
if ([regex]::Matches($installerSmoke, '(?m)^\$global:LASTEXITCODE = 0\s*$').Count -ne 1) {
    throw "Installer smoke must normalize the native exit code exactly once"
}

Assert-Match $release '(?m)^permissions:\s*\r?\n\s+contents:\s+read' "release default permissions must be contents: read"
Assert-Match $release '(?m)^on:\s*\r?\n\s+workflow_dispatch:' "release workflow must run only from workflow_dispatch"
if ($release -match '(?m)^\s{2}push:\s*$') { throw "release workflow must not run from an untrusted tag push" }
Assert-NoWorkflowContextInRun $release
Assert-Match $release 'publish:[\s\S]*?permissions:\s*\r?\n\s+contents:\s+write' "publish job alone must have contents: write"
if ([regex]::Matches($release, '(?m)^\s+contents:\s+write\s*$').Count -ne 1) {
    throw "release workflow must grant contents: write exactly once"
}
Assert-Match $release 'persist-credentials:\s*false' "release checkout must not persist credentials"
Assert-Match $release 'ref:\s*refs/tags/\$\{\{ inputs\.tag \}\}' "manual dispatch must check out an explicit tag ref"
Assert-Match $release '(?ms)^  build:.*?permissions:\s*\r?\n\s+actions:\s+read\s*\r?\n\s+contents:\s+read' "release build must have actions: read and contents: read"
Assert-Match $release '(?ms)^\s{4}outputs:\s*\r?\n\s+release_tag:\s*\$\{\{ steps\.release_meta\.outputs\.release_tag \}\}\s*\r?\n\s+tag_commit:\s*\$\{\{ steps\.release_meta\.outputs\.tag_commit \}\}' "release build must expose the validated tag commit to publish"
Assert-Match $release 'Validate trusted release target and successful CI' "release must validate trust before checkout"
Assert-Match $release 'WORKFLOW_REF:\s*\$\{\{ github\.ref \}\}' "release must receive the workflow source ref through env"
Assert-Match $release '\$env:WORKFLOW_REF -cne "refs/heads/\$env:DEFAULT_BRANCH"' "release must reject dispatches outside the trusted default branch"
Assert-Match $release 'compare/\$tagCommit\.\.\.\$env:DEFAULT_BRANCH' "release must verify tag ancestry through the trusted API"
Assert-Match $release 'actions/workflows/ci\.yml/runs\?head_sha=\$tagCommit&status=success' "release must require successful CI for the exact tag commit"
Assert-Match $release 'No successful CI run exists for tag commit' "release CI verification must fail closed"
Assert-Match $release '(?ms)Validate trusted release target and successful CI.*?uses:\s*actions/checkout@' "release trust checks must run before checkout"
Assert-Match $release '(?ms)Verify checked-out tag ancestry and release contract.*?npm ci' "release ancestry verification must run before package installation"
Assert-Match $release 'git rev-parse "refs/tags/\$env:RELEASE_TAG\^\{commit\}"' "release build must verify tag commit identity"
Assert-Match $release 'git merge-base --is-ancestor \$head "origin/\$env:DEFAULT_BRANCH"' "release build must verify origin default-branch ancestry"
Assert-Match $release 'environment:\s*release' "publish must use the protected release environment"
Assert-Match $release 'Revalidate release tag identity' "publish must revalidate the tag after environment approval"
Assert-Match $release 'EXPECTED_COMMIT:\s*\$\{\{ needs\.build\.outputs\.tag_commit \}\}' "publish must receive the validated tag commit"
Assert-Match $release '\$target\.sha -cne \$env:EXPECTED_COMMIT' "publish must reject a moved release tag"
Assert-Match $release 'compare/\$env:EXPECTED_COMMIT\.\.\.\$env:DEFAULT_BRANCH' "publish must revalidate default-branch ancestry for the expected commit"
Assert-Match $release '(?ms)^\s{6}- name:\s*Revalidate release tag identity.*?^\s{6}- name:\s*Create GitHub Release' "publish tag identity check must run before release creation"
Assert-Match $release 'Reject an existing release tag' "publish must reject an existing release"
Assert-Match $release 'overwrite_files:\s*false' "release assets must be immutable"
Assert-Match $release 'generate_release_notes:\s*false' "release must disable generated PR and commit notes"
Assert-Match $release 'body_path:\s*release-assets/release-notes\.md' "release must publish the reviewed Korean body file"
Assert-Match $release 'docs/releases/\$version\.md' "release build must stage the notes matching the product version"
if ($release -match '(?m)^\s+body:\s*(?:\||>)?\s*$') { throw "release workflow must not embed a fixed body" }
Assert-Match $release '(?m)^concurrency:' "release workflow must serialize each release tag"
Assert-Match $release 'npm test' "release workflow must run frontend tests"
Assert-Match $release 'npm run tauri:build' "release workflow must use tauri build"
Assert-Match $release 'TAURI_SIGNING_PRIVATE_KEY:\s*\$\{\{ secrets\.TAURI_SIGNING_PRIVATE_KEY \}\}' "release build must receive updater private key through secrets"
Assert-Match $release 'TAURI_SIGNING_PRIVATE_KEY_PASSWORD:\s*\$\{\{ secrets\.TAURI_SIGNING_PRIVATE_KEY_PASSWORD \}\}' "release build must receive updater key password through secrets"
if ($release -match '(?m)^\s*run:.*TAURI_SIGNING_PRIVATE_KEY' -or $release -match '\$env:TAURI_SIGNING_PRIVATE_KEY') {
    throw "Updater private key must never be referenced by workflow run source"
}
Assert-Match $release 'PreReleaseLabel\.Length' "all SemVer prerelease labels must be classified"
Assert-Match $release 'bdo-optimizer-launcher-setup\.exe' "release assets must use the NSIS installer"
Assert-Match $release 'bdo-optimizer-launcher-setup\.exe\.sig' "release must publish the updater signature"
Assert-Match $release 'release-assets/latest\.json' "release must publish the static updater manifest"
Assert-Match $release 'backend::update::tests::release_artifact_signature_matches_embedded_public_key --locked -- --ignored --exact' "release must run the exact artifact signature test against the embedded public key"
Assert-Match $release 'create_updater_manifest\.ps1' "release must generate latest.json through the validated script"
Assert-Match $release 'create_updater_manifest\.ps1[^\r\n]+-NotesPath docs/releases/\$version\.md' "updater manifest must use the reviewed versioned Korean release notes"
Assert-Match $updaterManifest 'notes\s*=\s*\$notes' "updater manifest must publish release notes for the in-app UI"
Assert-Match $release "foreach \(\`$name in @\('bdo-optimizer-launcher-setup\.exe', 'bdo-optimizer-launcher-setup\.exe\.sig', 'latest\.json', 'manual\.pdf'\)\)" "publish must reverify every updater asset hash"
Assert-Match $release 'check_tauri_embed\.ps1' "release workflow must verify embedded production assets"
Assert-Match $release 'check_tauri_embed\.ps1[^\r\n]+-ExpectedVersion\s+\$version[^\r\n]+-ExpectedExecutionLevel\s+requireAdministrator' "release embed check must verify version and elevation"
Assert-Match $release 'smoke_test_installer\.ps1' "release workflow must smoke test installer lifecycle"
Assert-Match $release '(?ms)^\s{6}- name: Smoke test install, upgrade, and uninstall cleanup\r?\n\s{8}timeout-minutes:\s*10\r?\n\s{8}shell:\s*pwsh\s*$' "installer smoke workflow step must have a ten-minute timeout"
Assert-Match $release 'smoke_test_installer\.ps1[^\r\n]*\r?\n\s*\$global:LASTEXITCODE = 0' "release workflow must normalize a successful legacy smoke script native exit code"
Assert-Match $installerSmoke 'Start-Process -FilePath \$file -ArgumentList \$arguments -PassThru -WindowStyle Hidden' "installer smoke must start the target process without descendant-tree waiting"
Assert-Match $installerSmoke '\$process\.WaitForExit\(\$TimeoutSeconds \* 1000\)' "installer smoke must bound the target PID wait"
Assert-Match $installerSmoke 'Stop-Process -Id \$process\.Id -Force' "installer smoke must terminate a timed-out target process"
Assert-Match $installerSmoke 'installer smoke phase: \$scenario' "installer smoke must identify the phase that hangs"
if ($installerSmoke -match '(?m)^\s*\$process\s*=\s*Start-Process[^\r\n]+-Wait(?:\s|$)') {
    throw "installer smoke must not wait for a relaunched resident process tree"
}
if ([regex]::Matches($release, '(?m)^\s+\$global:LASTEXITCODE = 0\s*$').Count -ne 1) {
    throw "release workflow must normalize the legacy smoke exit code exactly once"
}
Assert-Match $release '(?ms)^  build:.*?cargo install cargo-audit --version 0\.22\.2 --locked.*?run:\s*cargo audit\s*$.*?npm run tauri:build' "release build must run pinned cargo-audit before packaging"
if ([regex]::Matches($release, '(?m)^\s+run:\s+cargo install cargo-audit --version 0\.22\.2 --locked\s*$').Count -ne 1) {
    throw "release workflow must install pinned cargo-audit exactly once"
}
if ([regex]::Matches($release, '(?m)^\s+run:\s+cargo audit\s*$').Count -ne 1) {
    throw "release workflow must run cargo audit exactly once"
}
Assert-Match $release '(?ms)^  build:.*?run:\s*cargo install ripgrep --version 15\.1\.0 --locked\s*$.*?check_tauri_embed\.ps1' "release build must install pinned ripgrep before the production embed check"
if ([regex]::Matches($release, '(?m)^\s+run:\s+cargo install ripgrep --version 15\.1\.0 --locked\s*$').Count -ne 1) {
    throw "release workflow must install pinned ripgrep exactly once"
}
Assert-Match $release 'Verify downloaded release asset hashes' "publish job must reverify downloaded asset hashes"
if ($release -match '(?m)run:\s*cargo build --release') { throw "bare cargo release build is forbidden" }

Assert-Match $ci 'npm test' "CI must run frontend tests"
Assert-Match $ci 'cargo audit' "CI must run RustSec cargo audit"
Assert-Match $ci 'cargo-audit --version 0\.22\.2 --locked' "CI cargo-audit version must be pinned"
Assert-Match $ci 'test_updater_manifest\.ps1' "CI must run updater manifest behavioral tests"
Assert-Match $updaterManifest 'Repository -cne "Lv2dev/bdo-optimizer-launcher"' "updater manifest must pin the approved repository"
Assert-Match $updaterManifest 'https://github\.com/\$Repository/releases/download/\$Tag/' "updater manifest must use an HTTPS GitHub Release asset"
Assert-Match $updaterManifest 'windows-x86_64' "updater manifest must declare the Windows x64 platform"
Assert-Match $updaterManifestTest 'attacker/repo' "updater manifest tests must reject a foreign repository"
Assert-Match $updaterManifestTest 'tag/version pair' "updater manifest tests must reject tag/version mismatch"
Assert-Match $readme 'bdo-optimizer-launcher-setup\.exe' "README must document the installer"
Assert-Match $readme 'Get-FileHash\s+\.\\bdo-optimizer-launcher-setup\.exe\s+-Algorithm\s+SHA256' "README installer hash command missing"
Assert-Match $readme 'npm test' "README developer verification must include frontend tests"
Assert-Match $readme '<img\s+src="assets/app_256\.png"\s+width="112"\s+alt="BDO Optimizer Launcher 로고">' "README product logo is missing or inaccessible"
Assert-Match $readme '<a href="\.\./\.\./releases/latest"><strong>최신 버전 받기</strong></a>' "README latest release link is missing"
Assert-Match $readme '<a href="docs/distribution/manual\.pdf"><strong>사용 설명서</strong></a>' "README manual link is missing"
Assert-Match $readme '<img\s+src="docs/readme/overview\.png"\s+width="760"\s+alt="BDO Optimizer Launcher 제어 탭에서 게임 상태와 CPU 최적화 모드를 관리하는 화면">' "README overview image or Korean alt text is missing"
Assert-Match $readme '<img\s+src="docs/readme/app-tour\.gif"\s+width="720"\s+alt="BDO Optimizer Launcher의 제어, 스케줄, 모니터, 설정 탭을 순서대로 보여주는 제품 화면">' "README app tour or Korean alt text is missing"
Assert-Match $readme '(?s)assets/app_256\.png.*?\.\./\.\./releases/latest.*?docs/distribution/manual\.pdf.*?docs/readme/overview\.png.*?## 화면 둘러보기.*?docs/readme/app-tour\.gif' "README visual onboarding order changed"

$overviewPath = Join-Path $root "docs\readme\overview.png"
$tourPath = Join-Path $root "docs\readme\app-tour.gif"
foreach ($path in @($overviewPath, $tourPath)) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "README visual asset is missing: $([IO.Path]::GetRelativePath($root, $path))"
    }
    if ((Get-Item -LiteralPath $path).Length -ge 10MB) {
        throw "README visual asset must stay below 10 MB: $([IO.Path]::GetRelativePath($root, $path))"
    }
}
$overviewBytes = [IO.File]::ReadAllBytes($overviewPath)
if ($overviewBytes.Length -lt 8 -or [BitConverter]::ToString($overviewBytes[0..7]) -cne '89-50-4E-47-0D-0A-1A-0A') {
    throw "README overview.png is not a valid PNG"
}
$tourBytes = [IO.File]::ReadAllBytes($tourPath)
$tourHeader = if ($tourBytes.Length -ge 6) { [Text.Encoding]::ASCII.GetString($tourBytes, 0, 6) } else { "" }
if ($tourHeader -notin @('GIF87a', 'GIF89a')) {
    throw "README app-tour.gif is not a valid GIF"
}
if ((Get-Item -LiteralPath $overviewPath).Length + (Get-Item -LiteralPath $tourPath).Length -gt 8MB) {
    throw "README visual assets exceed the 8 MB combined budget"
}

Write-Host "release workflow checks passed for version $version"
