$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$script = Join-Path $root "scripts\create_updater_manifest.ps1"
$fixture = Join-Path $env:TEMP ("bdo-updater-manifest-" + [Guid]::NewGuid().ToString("N"))

function Invoke-Fixture([hashtable]$Overrides) {
    $parameters = @{
        Version = "0.3.0"
        Tag = "v0.3.0"
        Repository = "Lv2dev/bdo-optimizer-launcher"
        InstallerPath = Join-Path $fixture "bdo-optimizer-launcher-setup.exe"
        SignaturePath = Join-Path $fixture "bdo-optimizer-launcher-setup.exe.sig"
        NotesPath = Join-Path $fixture "release-notes.md"
        OutputPath = Join-Path $fixture "latest.json"
    }
    foreach ($key in $Overrides.Keys) { $parameters[$key] = $Overrides[$key] }
    & $script @parameters *> $null
}

function Assert-Rejected([hashtable]$Overrides, [string]$Name) {
    $rejected = $false
    try { Invoke-Fixture $Overrides }
    catch { $rejected = $true }
    if (-not $rejected) { throw "Updater manifest accepted invalid $Name" }
}

try {
    New-Item -ItemType Directory -Path $fixture -Force | Out-Null
    [IO.File]::WriteAllBytes(
        (Join-Path $fixture "bdo-optimizer-launcher-setup.exe"),
        [byte[]](0x4D, 0x5A, 0x00, 0x01)
    )
    Set-Content -LiteralPath (Join-Path $fixture "bdo-optimizer-launcher-setup.exe.sig") -Value "QUJDRA==" -Encoding ascii -NoNewline
    [IO.File]::WriteAllText(
        (Join-Path $fixture "release-notes.md"),
        "## 변경 사항`n- updater 안정성을 개선했습니다.`n",
        [Text.UTF8Encoding]::new($false)
    )

    Invoke-Fixture @{}
    $manifest = Get-Content -LiteralPath (Join-Path $fixture "latest.json") -Raw | ConvertFrom-Json
    $platform = $manifest.platforms.'windows-x86_64'
    if ([string]$manifest.version -cne "0.3.0") { throw "Updater manifest version mismatch" }
    if ([string]$manifest.notes -cne "## 변경 사항`n- updater 안정성을 개선했습니다.") { throw "Updater manifest notes mismatch" }
    if ([string]$platform.signature -cne "QUJDRA==") { throw "Updater manifest signature mismatch" }
    if ([string]$platform.url -cne "https://github.com/Lv2dev/bdo-optimizer-launcher/releases/download/v0.3.0/bdo-optimizer-launcher-setup.exe") {
        throw "Updater manifest URL mismatch"
    }

    Assert-Rejected @{ Repository = "attacker/repo" } "repository"
    Assert-Rejected @{ Tag = "v0.3.1" } "tag/version pair"
    Assert-Rejected @{ Version = "0.3"; Tag = "v0.3" } "SemVer"

    [IO.File]::WriteAllText((Join-Path $fixture "release-notes.md"), "  `n", [Text.UTF8Encoding]::new($false))
    Assert-Rejected @{} "empty release notes"
    [IO.File]::WriteAllText(
        (Join-Path $fixture "release-notes.md"),
        "## 변경 사항`n- updater 안정성을 개선했습니다.`n",
        [Text.UTF8Encoding]::new($false)
    )

    Set-Content -LiteralPath (Join-Path $fixture "bdo-optimizer-launcher-setup.exe.sig") -Value "not a signature" -Encoding ascii -NoNewline
    Assert-Rejected @{} "signature"

    Write-Host "updater manifest behavioral tests passed"
    $global:LASTEXITCODE = 0
}
finally {
    if (Test-Path -LiteralPath $fixture) {
        Remove-Item -LiteralPath $fixture -Recurse -Force
    }
}
