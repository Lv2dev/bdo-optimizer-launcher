param(
    [Parameter(Mandatory = $true)]
    [string]$Installer,
    [Parameter(Mandatory = $true)]
    [string]$ExpectedVersion,
    [switch]$AllowSystemChanges
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
if (-not $AllowSystemChanges) {
    throw "Installer smoke test changes Program Files and Task Scheduler. Pass -AllowSystemChanges explicitly."
}

$root = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot "installer_acl_guard.ps1")
$installerPath = if ([IO.Path]::IsPathRooted($Installer)) { $Installer } else { Join-Path $root $Installer }
if (-not (Test-Path -LiteralPath $installerPath -PathType Leaf)) { throw "Installer not found: $installerPath" }

$programFiles = if ($env:ProgramW6432) { $env:ProgramW6432 } else { $env:ProgramFiles }
$installDir = Join-Path $programFiles "BDO Optimizer"
$installedExe = Join-Path $installDir "bdo-optimizer-launcher.exe"
$uninstaller = Join-Path $installDir "uninstall.exe"
$uninstallKey = "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\BDO Optimizer"
$schtasks = Join-Path $env:SystemRoot "System32\schtasks.exe"
$icacls = Join-Path $env:SystemRoot "System32\icacls.exe"
$cmd = Join-Path $env:SystemRoot "System32\cmd.exe"
$taskNames = @("BDO_Optimizer_Launcher_Autostart", "BDO_Auto_Shutdown_Once", "BDO_Auto_Shutdown_Weekly")
$fixtureId = [Guid]::NewGuid().ToString("N")
$untrustedUninstallMarker = Join-Path $env:TEMP "bdo-untrusted-uninstaller-$fixtureId.txt"
$reparseTarget = Join-Path $env:TEMP "bdo-installer-reparse-target-$fixtureId"
$reparsePath = Join-Path $installDir "reparse-fixture"
$currentUserSid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$untrustedSids = @("S-1-1-0", "S-1-5-11", "S-1-5-32-545", $currentUserSid)

function Invoke-Checked([string]$file, [string[]]$arguments) {
    $process = Start-Process -FilePath $file -ArgumentList $arguments -Wait -PassThru -WindowStyle Hidden
    if ($process.ExitCode -ne 0) { throw "$file failed with exit code $($process.ExitCode)" }
}

function Invoke-ExpectedFailure([string]$file, [string[]]$arguments, [string]$scenario) {
    $process = Start-Process -FilePath $file -ArgumentList $arguments -Wait -PassThru -WindowStyle Hidden
    if ($process.ExitCode -eq 0) { throw "$scenario unexpectedly succeeded" }
}

function Get-InstalledProcesses {
    return @(
        Get-CimInstance Win32_Process -Filter "Name='bdo-optimizer-launcher.exe'" -ErrorAction SilentlyContinue |
            Where-Object { [string]$_.ExecutablePath -ieq $installedExe }
    )
}

function Wait-InstalledProcess([bool]$Present, [int]$TimeoutSeconds) {
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $processes = @(Get-InstalledProcesses)
        if (($Present -and $processes.Count -gt 0) -or (-not $Present -and $processes.Count -eq 0)) {
            return $processes
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Installed process presence did not become '$Present' within $TimeoutSeconds seconds."
}

function Test-Task([string]$name) {
    & $schtasks /Query /TN $name *> $null
    return $LASTEXITCODE -eq 0
}

function Test-DangerousUntrustedAce([string]$path) {
    return Test-SddlDangerousUntrustedAce (Get-InstallerAclSddl $path) $untrustedSids
}

function Assert-ProtectedPath([string]$path) {
    $item = Get-Item -LiteralPath $path -Force
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Installed path is a reparse point: $path" }
    $ownerSid = Get-SddlOwnerSid (Get-InstallerAclSddl $path)
    if ($ownerSid -ne "S-1-5-32-544") { throw "Installed path owner is not Administrators: $path" }
    if (Test-DangerousUntrustedAce $path) { throw "Installed path is writable by an untrusted SID: $path" }
}

function Assert-InstalledContract {
    if (-not (Test-Path -LiteralPath $installedExe -PathType Leaf)) { throw "Installed exe missing" }
    if (-not (Test-Path -LiteralPath $uninstaller -PathType Leaf)) { throw "Uninstaller missing" }
    if (-not (Test-Path -LiteralPath $uninstallKey)) { throw "Uninstall registry key missing" }
    $entry = Get-ItemProperty -LiteralPath $uninstallKey
    if ([string]$entry.DisplayVersion -ne $ExpectedVersion) { throw "DisplayVersion mismatch" }
    if ([string]$entry.InstallLocation.Trim('"') -ne $installDir) { throw "InstallLocation mismatch" }
    Assert-ProtectedPath $installDir
    Assert-ProtectedPath $installedExe
    Assert-ProtectedPath $uninstaller
}

if ((Test-Path -LiteralPath $installDir) -or (Test-Path -LiteralPath $uninstallKey)) {
    throw "Installer smoke collision: existing installation detected"
}
foreach ($name in $taskNames) {
    if (Test-Task $name) { throw "Installer smoke collision: existing task detected: $name" }
}

try {
    Invoke-Checked $installerPath @("/S")
    Assert-InstalledContract

    foreach ($name in $taskNames) {
        if (Test-Task $name) { throw "First install unexpectedly created task: $name" }
    }

    New-Item -ItemType Directory -Path $reparseTarget | Out-Null
    $targetAclBefore = (Get-Acl -LiteralPath $reparseTarget).Sddl
    New-Item -ItemType Junction -Path $reparsePath -Target $reparseTarget | Out-Null
    Invoke-ExpectedFailure $installerPath @("/S") "Installer with descendant reparse point"
    $targetAclAfter = (Get-Acl -LiteralPath $reparseTarget).Sddl
    if ($targetAclAfter -cne $targetAclBefore) { throw "Installer changed ACL through descendant reparse point" }
    Remove-Item -LiteralPath $reparsePath -Force

    foreach ($name in $taskNames) {
        & $schtasks /Create /TN $name /TR 'cmd.exe /c exit 0' /SC ONLOGON /F *> $null
        if ($LASTEXITCODE -ne 0) { throw "Failed to seed task: $name" }
    }

    $taskXmlBefore = @{}
    foreach ($name in $taskNames) {
        $taskXmlBefore[$name] = ((& $schtasks /Query /TN $name /XML) | Out-String).Trim()
    }
    Set-ItemProperty -LiteralPath $uninstallKey -Name DisplayVersion -Value "0.1.9"
    $unsafeUninstallString = "`"$cmd`" /D /C `"echo elevated>`"$untrustedUninstallMarker`" & rem`""
    Set-ItemProperty -LiteralPath $uninstallKey -Name UninstallString -Value $unsafeUninstallString
    & $icacls $installDir /grant "*$currentUserSid`:(OI)(CI)M" /T /Q *> $null
    if ($LASTEXITCODE -ne 0) { throw "Failed to seed user-writable installer ACL" }
    if (-not (Test-DangerousUntrustedAce $installDir)) { throw "Seeded user-writable ACL was not observable" }

    Invoke-Checked $installerPath @("/P", "/UPDATE", "/R", "/ARGS", "--minimized")
    $restartedProcesses = @(Wait-InstalledProcess $true 15)
    foreach ($process in $restartedProcesses) {
        Stop-Process -Id $process.ProcessId -Force -ErrorAction Stop
    }
    Wait-InstalledProcess $false 10 | Out-Null
    if (Test-Path -LiteralPath $untrustedUninstallMarker) {
        throw "Untrusted existing uninstaller was executed"
    }
    foreach ($name in $taskNames) {
        if (-not (Test-Task $name)) { throw "Upgrade removed task unexpectedly: $name" }
        $taskXmlAfter = ((& $schtasks /Query /TN $name /XML) | Out-String).Trim()
        if ($taskXmlAfter -cne $taskXmlBefore[$name]) { throw "Upgrade changed task unexpectedly: $name" }
    }
    Assert-InstalledContract

    $preDeletedTask = $taskNames[0]
    & $schtasks /Delete /TN $preDeletedTask /F *> $null
    if ($LASTEXITCODE -ne 0 -or (Test-Task $preDeletedTask)) {
        throw "Failed to prepare already-absent uninstall task case: $preDeletedTask"
    }
    Invoke-Checked $uninstaller @("/S")
    foreach ($name in $taskNames) {
        if (Test-Task $name) { throw "Uninstall left orphan task: $name" }
    }
    if (Test-Path -LiteralPath $installedExe) { throw "Uninstall left installed executable" }
    if (Test-Path -LiteralPath $uninstallKey) { throw "Uninstall left registry entry" }
} finally {
    if (Test-Path -LiteralPath $reparsePath) {
        Remove-Item -LiteralPath $reparsePath -Force
    }
    if (Test-Path -LiteralPath $reparseTarget) {
        Remove-Item -LiteralPath $reparseTarget -Recurse -Force
    }
    Remove-Item -LiteralPath $untrustedUninstallMarker -Force -ErrorAction SilentlyContinue
    if (Test-Path -LiteralPath $uninstaller -PathType Leaf) {
        Start-Process -FilePath $uninstaller -ArgumentList @("/S") -Wait -WindowStyle Hidden
    }
    foreach ($name in $taskNames) {
        & $schtasks /Delete /TN $name /F *> $null
    }
}
Write-Host "installer install/upgrade/uninstall smoke test passed"
$global:LASTEXITCODE = 0
