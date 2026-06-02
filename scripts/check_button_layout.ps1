$ErrorActionPreference = "Stop"

# Verify split slint files (M35 layout). ASCII-only messages and no backtick
# line continuation (Windows PowerShell 5.x compatibility).
# Korean string literals in slint UI are checked indirectly via callback/property
# names, not via embedded Korean regex (see memory.md 2026-05-16).

$root  = Split-Path -Parent $PSScriptRoot
$uiDir = Join-Path $root "ui"

$utf8Bom = New-Object System.Text.UTF8Encoding $true

function Read-Slint([string]$relativePath) {
    $path = Join-Path $uiDir $relativePath
    if (-not (Test-Path $path)) { throw "Missing slint file: $relativePath" }
    # PowerShell 5.x: String.StartsWith([char]0xFEFF) is culture-aware and
    # treats BOM as zero-width, returning True for any string. Check raw bytes
    # instead so the BOM strip only happens when the file actually has one.
    $bytes = [System.IO.File]::ReadAllBytes($path)
    $text  = [System.IO.File]::ReadAllText($path, $utf8Bom)
    if ($bytes.Length -ge 3 -and $bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF) {
        if ($text.Length -gt 0 -and [int]$text[0] -eq 0xFEFF) {
            $text = $text.Substring(1)
        }
    }
    return $text
}

function Assert-Match([string]$content, [string]$pattern, [string]$message) {
    if ($content -notmatch $pattern) { throw $message }
}

# 1. ui/theme.slint -- enum 3 (incl. OptimizeMode.low_power from M49 rename)
$theme = Read-Slint "theme.slint"
Assert-Match $theme 'export\s+enum\s+ThemeMode\s*\{\s*light\s*,\s*dark\s*,\s*system\s*\}' "ThemeMode enum missing"
Assert-Match $theme 'export\s+enum\s+RuleKind\s*\{\s*daily\s*,\s*weekday\s*,\s*weekend\s*,\s*specific\s*\}' "RuleKind enum missing"
Assert-Match $theme 'export\s+enum\s+OptimizeMode\s*\{\s*high\s*,\s*normal\s*,\s*low_power\s*\}' "OptimizeMode enum must have high/normal/low_power (M49 rename)"

# 2. ui/widgets.slint -- CheckIcon + Sparkline(M36) + SectionHeader
$widgets = Read-Slint "widgets.slint"
Assert-Match $widgets 'export\s+component\s+CheckIcon\s+inherits\s+Rectangle' "CheckIcon definition missing"
Assert-Match $widgets 'export\s+component\s+Sparkline\s+inherits\s+Rectangle' "Sparkline(M36) definition missing"
Assert-Match $widgets 'export\s+component\s+SectionHeader' "SectionHeader definition missing"

# 3. ui/buttons.slint -- 7 components + arithmetic widths + 3-button(M33) + WeekdayChip(M35)
$buttons = Read-Slint "buttons.slint"
Assert-Match $buttons 'export\s+component\s+DBtn\s+inherits\s+Rectangle' "DBtn missing"
Assert-Match $buttons 'export\s+component\s+TwoButtonRow\s+inherits\s+Rectangle' "TwoButtonRow missing"
Assert-Match $buttons 'export\s+component\s+ToggleSwitch\s+inherits\s+Rectangle' "ToggleSwitch missing"
Assert-Match $buttons 'export\s+component\s+WeekdayChip\s+inherits\s+Rectangle' "WeekdayChip(M35) missing"
Assert-Match $buttons 'export\s+component\s+TabBtn\s+inherits\s+Rectangle' "TabBtn missing"
Assert-Match $buttons 'export\s+component\s+BadgedDBtn\s+inherits\s+Rectangle' "BadgedDBtn missing"
Assert-Match $buttons 'export\s+component\s+OptimizeRow\s+inherits\s+Rectangle' "OptimizeRow missing"
Assert-Match $buttons 'preferred-width:\s*0px' "DBtn must keep preferred-width: 0px"
Assert-Match $buttons 'width:\s*\(root\.width\s*-\s*root\.gap\)\s*/\s*2' "TwoButtonRow arithmetic width (root.width - root.gap)/2 missing"
Assert-Match $buttons 'col-w:\s*\(root\.width\s*-\s*16px\)\s*/\s*3' "OptimizeRow M33 three-button arithmetic width missing"
Assert-Match $buttons 'col-1-x:' "OptimizeRow col-1-x property missing"
Assert-Match $buttons 'col-2-x:' "OptimizeRow col-2-x property missing"
Assert-Match $buttons 'col-3-x:' "OptimizeRow col-3-x property missing"
Assert-Match $buttons 'callback\s+high-clicked' "OptimizeRow high-clicked callback missing"
Assert-Match $buttons 'callback\s+normal-clicked' "OptimizeRow normal-clicked(M33) callback missing"
Assert-Match $buttons 'callback\s+low-power-clicked' "OptimizeRow low-power-clicked callback missing"
Assert-Match $buttons 'ripple-opacity' "OptimizeRow ripple-opacity property missing"
Assert-Match $buttons '(?s)OptimizeRow[\s\S]*?clip:\s*true' "OptimizeRow clip: true missing"

# 4. ui/app.slint -- M35 tab order + Ctrl+1~4 + status bar wrap
$app = Read-Slint "app.slint"
$tabBtnMatches = [regex]::Matches($app, '(?s)TabBtn\s*\{[\s\S]*?active-tab\s*==\s*(\d)[\s\S]*?clicked\s*=>\s*\{\s*root\.active-tab\s*=\s*(\d)')
if ($tabBtnMatches.Count -lt 4) { throw "AppWindow must have 4 TabBtn declarations (found $($tabBtnMatches.Count))" }
$seenIndices = @()
foreach ($m in $tabBtnMatches) {
    $activeN = [int]$m.Groups[1].Value
    $clickN  = [int]$m.Groups[2].Value
    if ($activeN -ne $clickN) { throw "TabBtn active index $activeN must match clicked target $clickN" }
    $seenIndices += $activeN
}
$sorted = ($seenIndices | Sort-Object)
if (($sorted -join ',') -ne '0,1,2,3') { throw "Tab indices must be exactly 0..3 (M35 layout), found: $($seenIndices -join ',')" }

Assert-Match $app 'active-tab\s*==\s*2' "active-tab == 2 binding missing"
Assert-Match $app 'active-tab\s*==\s*3' "active-tab == 3 binding missing"
Assert-Match $app 'key-pressed\(event\)' "AppWindow FocusScope key-pressed handler missing"

$statusBlock = $null
if ($app -match '(?s)text:\s*root\.status-text;[\s\S]{0,400}') { $statusBlock = $Matches[0] }
if (-not $statusBlock -or $statusBlock -notmatch 'wrap:\s*word-wrap') { throw "status-text must use wrap: word-wrap" }
if ($statusBlock -match 'overflow:\s*elide') { throw "status-text must not use overflow: elide (would clip long messages)" }

$cbs = @('apply-high-mode','apply-normal-mode','apply-low-power-mode','launch-game','refresh-game-status','register-shutdown','cancel-once-shutdown','cancel-weekly-shutdown','toggle-mon-cores-view')
foreach ($cb in $cbs) {
    Assert-Match $app ("callback\s+{0}" -f [regex]::Escape($cb)) "AppWindow callback $cb missing"
}

# 5. ui/tabs/control.slint -- game action TwoButtonRow + mode OptimizeRow
$ctrl = Read-Slint "tabs/control.slint"
Assert-Match $ctrl 'export\s+component\s+ControlTab\s+inherits\s+ScrollView' "ControlTab missing"
Assert-Match $ctrl '(?s)TwoButtonRow\s*\{[\s\S]*?left-clicked\s*=>\s*\{\s*root\.refresh-game-status' "control: TwoButtonRow left-clicked must call refresh-game-status"
Assert-Match $ctrl '(?s)TwoButtonRow\s*\{[\s\S]*?right-clicked\s*=>\s*\{\s*root\.launch-game' "control: TwoButtonRow right-clicked must call launch-game"
Assert-Match $ctrl '(?s)OptimizeRow\s*\{[\s\S]*?high-clicked\s*=>\s*\{\s*root\.apply-high-mode' "control: OptimizeRow high-clicked must call apply-high-mode"
Assert-Match $ctrl '(?s)OptimizeRow\s*\{[\s\S]*?normal-clicked\s*=>\s*\{\s*root\.apply-normal-mode' "control: OptimizeRow normal-clicked must call apply-normal-mode (M33)"
Assert-Match $ctrl '(?s)OptimizeRow\s*\{[\s\S]*?low-power-clicked\s*=>\s*\{\s*root\.apply-low-power-mode' "control: OptimizeRow low-power-clicked must call apply-low-power-mode (M49)"

# 6. ui/tabs/schedule.slint -- accordion order + segmented + WeekdayChip x 7
$sch = Read-Slint "tabs/schedule.slint"
Assert-Match $sch 'export\s+component\s+ScheduleTab\s+inherits\s+ScrollView' "ScheduleTab missing"

$autoIdx = $sch.IndexOf('root.auto-mode-expanded = !root.auto-mode-expanded')
$sdIdx   = $sch.IndexOf('root.shutdown-expanded = !root.shutdown-expanded')
if ($autoIdx -lt 0 -or $sdIdx -lt 0) { throw "schedule: accordion header toggles missing" }
if ($autoIdx -gt $sdIdx) { throw "schedule: auto-mode accordion must appear before shutdown accordion" }

Assert-Match $sch 'auto-content\s*:=' "schedule: auto-content child id missing (P4 height pattern)"
Assert-Match $sch 'shutdown-content\s*:=' "schedule: shutdown-content child id missing (P4 height pattern)"
Assert-Match $sch 'auto-content\.preferred-height' "schedule: auto accordion must bind to child preferred-height"
Assert-Match $sch 'shutdown-content\.preferred-height' "schedule: shutdown accordion must bind to child preferred-height"
Assert-Match $sch 'root\.shutdown-weekly\s*=\s*false' "schedule: once segmented toggle missing"
Assert-Match $sch 'root\.shutdown-weekly\s*=\s*true' "schedule: weekly segmented toggle missing"

$chips = [regex]::Matches($sch, 'WeekdayChip\s*\{[^}]*selected\s*<=>\s*root\.shutdown-(mon|tue|wed|thu|fri|sat|sun)')
if ($chips.Count -lt 7) { throw "schedule: 7 WeekdayChip(M35) with two-way bind required (found $($chips.Count))" }

Assert-Match $sch '(?s)TwoButtonRow\s*\{[\s\S]*?left-clicked\s*=>\s*\{\s*root\.register-shutdown' "schedule: TwoButtonRow left-clicked must call register-shutdown"
Assert-Match $sch '(?s)TwoButtonRow\s*\{[\s\S]*?right-clicked\s*=>\s*\{[\s\S]*?root\.cancel-weekly-shutdown[\s\S]*?root\.cancel-once-shutdown' "schedule: TwoButtonRow right-clicked must route to cancel-weekly-shutdown / cancel-once-shutdown by weekly toggle (M46)"

# 7. ui/tabs/monitor.slint -- Sparkline(M36) + cores toggle
$mon = Read-Slint "tabs/monitor.slint"
Assert-Match $mon 'export\s+component\s+MonitorTab\s+inherits\s+ScrollView' "MonitorTab missing"
$sparkCount = ([regex]::Matches($mon, 'Sparkline\s*\{')).Count
if ($sparkCount -lt 1) { throw "MonitorTab must contain at least one Sparkline(M36)" }
Assert-Match $mon 'mon-cores-view' "monitor: mon-cores-view property missing"
Assert-Match $mon 'toggle-mon-cores-view' "monitor: toggle-mon-cores-view callback missing"

# 8. ui/tabs/settings.slint -- basic shape
$set = Read-Slint "tabs/settings.slint"
Assert-Match $set 'export\s+component\s+SettingsTab\s+inherits\s+ScrollView' "SettingsTab missing"

Write-Host "button layout check passed"
