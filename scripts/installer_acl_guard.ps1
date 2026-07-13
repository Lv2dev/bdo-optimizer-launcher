Set-StrictMode -Version Latest

function ConvertFrom-InstallerSddl([string]$Sddl) {
    return [Security.AccessControl.RawSecurityDescriptor]::new($Sddl)
}

function Get-SddlOwnerSid([string]$Sddl) {
    $descriptor = ConvertFrom-InstallerSddl $Sddl
    if ($null -eq $descriptor.Owner) { return $null }
    return $descriptor.Owner.Value
}

function Test-SddlDangerousUntrustedAce([string]$Sddl, [string[]]$UntrustedSids) {
    $descriptor = ConvertFrom-InstallerSddl $Sddl
    if ($null -eq $descriptor.DiscretionaryAcl) { return $true }

    $dangerousRights = [Security.AccessControl.FileSystemRights]::WriteData `
        -bor [Security.AccessControl.FileSystemRights]::AppendData `
        -bor [Security.AccessControl.FileSystemRights]::WriteExtendedAttributes `
        -bor [Security.AccessControl.FileSystemRights]::WriteAttributes `
        -bor [Security.AccessControl.FileSystemRights]::DeleteSubdirectoriesAndFiles `
        -bor [Security.AccessControl.FileSystemRights]::Delete `
        -bor [Security.AccessControl.FileSystemRights]::ChangePermissions `
        -bor [Security.AccessControl.FileSystemRights]::TakeOwnership
    $genericWrite = 0x40000000
    $genericAll = 0x10000000

    foreach ($ace in $descriptor.DiscretionaryAcl) {
        if ($ace -isnot [Security.AccessControl.QualifiedAce]) { continue }
        if ($ace.AceQualifier -ne [Security.AccessControl.AceQualifier]::AccessAllowed) { continue }
        if ($ace.AceFlags -band [Security.AccessControl.AceFlags]::InheritOnly) { continue }
        if ($ace.SecurityIdentifier.Value -notin $UntrustedSids) { continue }

        $mask = $ace.AccessMask
        if (
            ($mask -band [int]$dangerousRights) -ne 0 -or
            ($mask -band $genericWrite) -ne 0 -or
            ($mask -band $genericAll) -ne 0
        ) {
            return $true
        }
    }
    return $false
}

function Get-InstallerAclSddl([string]$Path) {
    $acl = Get-Acl -LiteralPath $Path
    $sections = [Security.AccessControl.AccessControlSections]::Owner `
        -bor [Security.AccessControl.AccessControlSections]::Access
    return $acl.GetSecurityDescriptorSddlForm($sections)
}
