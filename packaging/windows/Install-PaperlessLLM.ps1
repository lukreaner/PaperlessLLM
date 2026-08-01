#requires -Version 5.1

[CmdletBinding()]
param(
    [switch] $DesktopShortcut,
    [switch] $NoDesktopShortcut
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$ApplicationId = 'io.paperlessllm.PaperlessLLM'
$ApplicationName = 'PaperlessLLM'
$ManagedFileNames = @(
    'PaperlessLLM.exe',
    'paperless-llm.exe',
    'LICENSE',
    'README-Windows.txt',
    'VERSION',
    'SHA256SUMS',
    'Uninstall-PaperlessLLM.ps1',
    'Uninstall PaperlessLLM.cmd'
)

function Stop-WithMessage {
    param([Parameter(Mandatory = $true)][string] $Message)

    throw "PaperlessLLM installer: $Message"
}

function Get-FullPath {
    param([Parameter(Mandatory = $true)][string] $Path)

    return [System.IO.Path]::GetFullPath($Path).TrimEnd(
        [System.IO.Path]::DirectorySeparatorChar,
        [System.IO.Path]::AltDirectorySeparatorChar
    )
}

function Test-SamePath {
    param(
        [Parameter(Mandatory = $true)][string] $First,
        [Parameter(Mandatory = $true)][string] $Second
    )

    return [string]::Equals(
        (Get-FullPath -Path $First),
        (Get-FullPath -Path $Second),
        [System.StringComparison]::OrdinalIgnoreCase
    )
}

function Assert-OrdinaryDirectory {
    param([Parameter(Mandatory = $true)][string] $Path)

    if (-not (Test-Path -LiteralPath $Path)) {
        return
    }
    $item = Get-Item -LiteralPath $Path -Force
    if (-not $item.PSIsContainer) {
        Stop-WithMessage "the destination is not a directory: $Path"
    }
    if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
        Stop-WithMessage "the destination must not be a junction or symbolic link: $Path"
    }
}

function Read-ReleaseVersion {
    param([Parameter(Mandatory = $true)][string] $Path)

    $value = (Get-Content -LiteralPath $Path -Raw).Trim()
    if ($value -notmatch '^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$') {
        Stop-WithMessage 'the release contains an invalid VERSION file'
    }
    return $value
}

function Assert-ReleaseChecksums {
    param(
        [Parameter(Mandatory = $true)][string] $ReleaseDirectory,
        [Parameter(Mandatory = $true)][string] $ManifestPath
    )

    $expected = @{}
    $lines = @(Get-Content -LiteralPath $ManifestPath)
    foreach ($line in $lines) {
        if ($line -notmatch '^([0-9a-fA-F]{64})  (PaperlessLLM\.exe|paperless-llm\.exe)$') {
            Stop-WithMessage 'SHA256SUMS contains an unexpected or malformed entry'
        }
        $fileName = $Matches[2]
        if ($expected.ContainsKey($fileName)) {
            Stop-WithMessage "SHA256SUMS contains a duplicate entry for $fileName"
        }
        $expected[$fileName] = $Matches[1].ToUpperInvariant()
    }

    foreach ($fileName in @('PaperlessLLM.exe', 'paperless-llm.exe')) {
        if (-not $expected.ContainsKey($fileName)) {
            Stop-WithMessage "SHA256SUMS does not contain $fileName"
        }
        $actual = (Get-FileHash -LiteralPath (Join-Path $ReleaseDirectory $fileName) -Algorithm SHA256).Hash
        if (-not [string]::Equals($actual, $expected[$fileName], [System.StringComparison]::OrdinalIgnoreCase)) {
            Stop-WithMessage "$fileName failed its SHA-256 integrity check; download the release again"
        }
    }

    if ($expected.Count -ne 2) {
        Stop-WithMessage 'SHA256SUMS must contain exactly the two PaperlessLLM executables'
    }
}

function Assert-ManagedInstallation {
    param([Parameter(Mandatory = $true)][string] $InstallDirectory)

    $markerPath = Join-Path $InstallDirectory '.paperlessllm-install.json'
    if (-not (Test-Path -LiteralPath $markerPath -PathType Leaf)) {
        $children = @(Get-ChildItem -LiteralPath $InstallDirectory -Force)
        if ($children.Count -ne 0) {
            Stop-WithMessage "the install directory is not empty and was not created by PaperlessLLM: $InstallDirectory"
        }
        return
    }

    try {
        $marker = Get-Content -LiteralPath $markerPath -Raw | ConvertFrom-Json
    }
    catch {
        Stop-WithMessage 'the existing installation marker is invalid'
    }
    if ($marker.applicationId -ne $ApplicationId -or [int]$marker.schema -ne 1) {
        Stop-WithMessage 'the existing installation marker does not belong to PaperlessLLM'
    }
}

function Assert-ApplicationStopped {
    param([Parameter(Mandatory = $true)][string[]] $ExecutablePaths)

    $normalizedPaths = @($ExecutablePaths | ForEach-Object { Get-FullPath -Path $_ })
    $running = @()
    foreach ($processName in @('PaperlessLLM', 'paperless-llm')) {
        foreach ($process in @(Get-Process -Name $processName -ErrorAction SilentlyContinue)) {
            try {
                $processPath = $process.Path
                if ($null -ne $processPath -and ($normalizedPaths -contains (Get-FullPath -Path $processPath))) {
                    $running += $process.Id
                }
            }
            catch {
                # A process that exits during inspection is already safe to ignore.
            }
        }
    }
    if ($running.Count -ne 0) {
        Stop-WithMessage 'PaperlessLLM is running. Use Quit PaperlessLLM in the dashboard, then run the installer again.'
    }
}

function Get-ShortcutTarget {
    param([Parameter(Mandatory = $true)][string] $ShortcutPath)

    $shell = New-Object -ComObject WScript.Shell
    try {
        return $shell.CreateShortcut($ShortcutPath).TargetPath
    }
    finally {
        [void][System.Runtime.InteropServices.Marshal]::FinalReleaseComObject($shell)
    }
}

function Install-Shortcut {
    param(
        [Parameter(Mandatory = $true)][string] $ShortcutPath,
        [Parameter(Mandatory = $true)][string] $ExecutablePath,
        [Parameter(Mandatory = $true)][string] $WorkingDirectory
    )

    $shortcutDirectory = Split-Path -Parent $ShortcutPath
    [void](New-Item -ItemType Directory -Path $shortcutDirectory -Force)
    Assert-OrdinaryDirectory -Path $shortcutDirectory

    if (Test-Path -LiteralPath $ShortcutPath -PathType Leaf) {
        $existingTarget = Get-ShortcutTarget -ShortcutPath $ShortcutPath
        if ([string]::IsNullOrWhiteSpace($existingTarget) -or -not (Test-SamePath -First $existingTarget -Second $ExecutablePath)) {
            Stop-WithMessage "a shortcut not managed by this PaperlessLLM installation already exists: $ShortcutPath"
        }
    }

    $temporaryShortcut = Join-Path $shortcutDirectory ('.PaperlessLLM-{0}.lnk' -f [guid]::NewGuid().ToString('N'))
    $shell = New-Object -ComObject WScript.Shell
    try {
        $shortcut = $shell.CreateShortcut($temporaryShortcut)
        $shortcut.TargetPath = $ExecutablePath
        $shortcut.Arguments = ''
        $shortcut.WorkingDirectory = $WorkingDirectory
        $shortcut.Description = 'Start PaperlessLLM and open the dashboard'
        $shortcut.IconLocation = "$ExecutablePath,0"
        $shortcut.WindowStyle = 1
        $shortcut.Save()
    }
    finally {
        [void][System.Runtime.InteropServices.Marshal]::FinalReleaseComObject($shell)
    }

    try {
        Move-Item -LiteralPath $temporaryShortcut -Destination $ShortcutPath -Force
    }
    finally {
        if (Test-Path -LiteralPath $temporaryShortcut) {
            Remove-Item -LiteralPath $temporaryShortcut -Force
        }
    }
}

function Move-ManagedShortcutToBackup {
    param(
        [Parameter(Mandatory = $true)][string] $ShortcutPath,
        [Parameter(Mandatory = $true)][string] $ExecutablePath
    )

    if (-not (Test-Path -LiteralPath $ShortcutPath)) {
        return $null
    }
    if (-not (Test-Path -LiteralPath $ShortcutPath -PathType Leaf)) {
        Stop-WithMessage "the shortcut path is not a file: $ShortcutPath"
    }

    $existingTarget = Get-ShortcutTarget -ShortcutPath $ShortcutPath
    if ([string]::IsNullOrWhiteSpace($existingTarget) -or -not (Test-SamePath -First $existingTarget -Second $ExecutablePath)) {
        Stop-WithMessage "a shortcut not managed by this PaperlessLLM installation already exists: $ShortcutPath"
    }

    $shortcutDirectory = Split-Path -Parent $ShortcutPath
    $shortcutName = Split-Path -Leaf $ShortcutPath
    $backupPath = Join-Path $shortcutDirectory ('.{0}.rollback-{1}' -f $shortcutName, [guid]::NewGuid().ToString('N'))
    Move-Item -LiteralPath $ShortcutPath -Destination $backupPath
    return $backupPath
}

function Restore-ManagedShortcut {
    param(
        [Parameter(Mandatory = $true)][AllowNull()][string] $ShortcutPath,
        [Parameter(Mandatory = $true)][string] $ExecutablePath,
        [Parameter(Mandatory = $true)][bool] $MutationStarted,
        [AllowNull()][string] $BackupPath
    )

    if (-not $MutationStarted) {
        return
    }

    if (Test-Path -LiteralPath $ShortcutPath) {
        if (-not (Test-Path -LiteralPath $ShortcutPath -PathType Leaf)) {
            Stop-WithMessage "cannot roll back a shortcut because its path is no longer a file: $ShortcutPath"
        }
        $currentTarget = Get-ShortcutTarget -ShortcutPath $ShortcutPath
        if ([string]::IsNullOrWhiteSpace($currentTarget) -or -not (Test-SamePath -First $currentTarget -Second $ExecutablePath)) {
            Stop-WithMessage "cannot roll back a shortcut that is no longer managed by PaperlessLLM: $ShortcutPath"
        }
        Remove-Item -LiteralPath $ShortcutPath -Force
    }

    if (-not [string]::IsNullOrWhiteSpace($BackupPath)) {
        if (-not (Test-Path -LiteralPath $BackupPath -PathType Leaf)) {
            Stop-WithMessage "cannot roll back the missing shortcut backup: $BackupPath"
        }
        Move-Item -LiteralPath $BackupPath -Destination $ShortcutPath
    }
}

function Move-UninstallRegistrationToBackup {
    param([Parameter(Mandatory = $true)][string] $RegistryPath)

    if (-not (Test-Path -LiteralPath $RegistryPath)) {
        return $null
    }

    $parentPath = Split-Path -Parent $RegistryPath
    $backupName = '.PaperlessLLM.rollback-{0}' -f [guid]::NewGuid().ToString('N')
    $backupPath = Join-Path $parentPath $backupName
    Rename-Item -LiteralPath $RegistryPath -NewName $backupName
    return $backupPath
}

function Restore-UninstallRegistration {
    param(
        [Parameter(Mandatory = $true)][string] $RegistryPath,
        [Parameter(Mandatory = $true)][bool] $MutationStarted,
        [AllowNull()][string] $BackupPath
    )

    if (-not $MutationStarted) {
        return
    }

    if (Test-Path -LiteralPath $RegistryPath) {
        Remove-Item -LiteralPath $RegistryPath -Recurse -Force
    }

    if (-not [string]::IsNullOrWhiteSpace($BackupPath)) {
        if (-not (Test-Path -LiteralPath $BackupPath)) {
            Stop-WithMessage "cannot roll back the missing Apps registration backup: $BackupPath"
        }
        Rename-Item -LiteralPath $BackupPath -NewName (Split-Path -Leaf $RegistryPath)
    }
}

if ($DesktopShortcut -and $NoDesktopShortcut) {
    Stop-WithMessage '-DesktopShortcut and -NoDesktopShortcut cannot be used together'
}

$releaseDirectory = Get-FullPath -Path $PSScriptRoot
$requiredReleaseFiles = $ManagedFileNames + @('Install-PaperlessLLM.ps1')
foreach ($fileName in $requiredReleaseFiles) {
    $path = Join-Path $releaseDirectory $fileName
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        Stop-WithMessage "the release is incomplete; missing $fileName"
    }
}

$version = Read-ReleaseVersion -Path (Join-Path $releaseDirectory 'VERSION')
Assert-ReleaseChecksums `
    -ReleaseDirectory $releaseDirectory `
    -ManifestPath (Join-Path $releaseDirectory 'SHA256SUMS')

$localApplicationData = [Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)
$startMenuPrograms = [Environment]::GetFolderPath([Environment+SpecialFolder]::Programs)
$desktopDirectory = [Environment]::GetFolderPath([Environment+SpecialFolder]::DesktopDirectory)
if ([string]::IsNullOrWhiteSpace($localApplicationData) -or [string]::IsNullOrWhiteSpace($startMenuPrograms)) {
    Stop-WithMessage 'Windows did not provide the current user application directories'
}

$installDirectory = Get-FullPath -Path (Join-Path $localApplicationData 'Programs\PaperlessLLM')
$installParent = Split-Path -Parent $installDirectory
$startMenuShortcut = Join-Path $startMenuPrograms 'PaperlessLLM.lnk'
$desktopShortcutPath = if ([string]::IsNullOrWhiteSpace($desktopDirectory)) {
    $null
}
else {
    Join-Path $desktopDirectory 'PaperlessLLM.lnk'
}

[void](New-Item -ItemType Directory -Path $installParent -Force)
Assert-OrdinaryDirectory -Path $installParent
if (Test-Path -LiteralPath $installDirectory) {
    Assert-OrdinaryDirectory -Path $installDirectory
    Assert-ManagedInstallation -InstallDirectory $installDirectory
}
else {
    [void](New-Item -ItemType Directory -Path $installDirectory)
}

$guiExecutable = Join-Path $installDirectory 'PaperlessLLM.exe'
$consoleExecutable = Join-Path $installDirectory 'paperless-llm.exe'
Assert-ApplicationStopped -ExecutablePaths @($guiExecutable, $consoleExecutable)

$stagingDirectory = Join-Path $installParent ('.PaperlessLLM-stage-{0}' -f [guid]::NewGuid().ToString('N'))
[void](New-Item -ItemType Directory -Path $stagingDirectory)
$backups = @{}
$installedFiles = @()
$installationCommitted = $false
$startMenuShortcutBackup = $null
$desktopShortcutBackup = $null
$startMenuShortcutMutated = $false
$desktopShortcutMutated = $false
$uninstallRegistryPath = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\PaperlessLLM'
$uninstallRegistryBackup = $null
$uninstallRegistryMutationStarted = $false
try {
    foreach ($fileName in $ManagedFileNames) {
        Copy-Item `
            -LiteralPath (Join-Path $releaseDirectory $fileName) `
            -Destination (Join-Path $stagingDirectory $fileName)
    }

    $existingMarkerPath = Join-Path $installDirectory '.paperlessllm-install.json'
    $hadDesktopShortcut = $false
    if (Test-Path -LiteralPath $existingMarkerPath -PathType Leaf) {
        try {
            $existingMarker = Get-Content -LiteralPath $existingMarkerPath -Raw | ConvertFrom-Json
            $hadDesktopShortcut = [bool]$existingMarker.desktopShortcut
        }
        catch {
            Stop-WithMessage 'the existing installation marker is invalid'
        }
    }
    $createDesktopShortcut = if ($DesktopShortcut) {
        $true
    }
    elseif ($NoDesktopShortcut) {
        $false
    }
    else {
        $hadDesktopShortcut
    }

    $marker = [ordered]@{
        schema = 1
        applicationId = $ApplicationId
        version = $version
        desktopShortcut = $createDesktopShortcut
    }
    $marker | ConvertTo-Json | Set-Content `
        -LiteralPath (Join-Path $stagingDirectory '.paperlessllm-install.json') `
        -Encoding UTF8

    foreach ($fileName in ($ManagedFileNames + @('.paperlessllm-install.json'))) {
        $destination = Join-Path $installDirectory $fileName
        if (Test-Path -LiteralPath $destination) {
            $backup = Join-Path $installDirectory ('.{0}.backup-{1}' -f $fileName, [guid]::NewGuid().ToString('N'))
            Move-Item -LiteralPath $destination -Destination $backup
            $backups[$destination] = $backup
        }
        Move-Item -LiteralPath (Join-Path $stagingDirectory $fileName) -Destination $destination
        $installedFiles += $destination
    }

    # Keep the old Apps registration intact under a temporary name until every
    # other installation step has succeeded. Renaming the key preserves value
    # types, unknown values, and subkeys exactly if an upgrade must roll back.
    $uninstallRegistryBackup = Move-UninstallRegistrationToBackup -RegistryPath $uninstallRegistryPath
    $uninstallRegistryMutationStarted = $true
    [void](New-Item -Path $uninstallRegistryPath -Force)
    $uninstallScript = Join-Path $installDirectory 'Uninstall-PaperlessLLM.ps1'
    $quotedUninstallScript = '"{0}"' -f $uninstallScript
    $uninstallCommand = "powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File $quotedUninstallScript"
    $estimatedBytes = 0L
    foreach ($fileName in $ManagedFileNames) {
        $estimatedBytes += (Get-Item -LiteralPath (Join-Path $installDirectory $fileName)).Length
    }
    $registryValues = [ordered]@{
        DisplayName = $ApplicationName
        DisplayVersion = $version
        DisplayIcon = ('"{0}",0' -f $guiExecutable)
        Publisher = 'PaperlessLLM contributors'
        InstallLocation = $installDirectory
        UninstallString = $uninstallCommand
        QuietUninstallString = "$uninstallCommand -Quiet"
        URLInfoAbout = 'https://github.com/lukreaner/PaperlessLLM'
        NoModify = 1
        NoRepair = 1
        EstimatedSize = [Math]::Max(1, [Math]::Ceiling($estimatedBytes / 1KB))
    }
    foreach ($entry in $registryValues.GetEnumerator()) {
        $propertyType = if ($entry.Value -is [int] -or $entry.Value -is [long] -or $entry.Value -is [double]) {
            'DWord'
        }
        else {
            'String'
        }
        $propertyValue = if ($propertyType -eq 'DWord') {
            [int]$entry.Value
        }
        else {
            [string]$entry.Value
        }
        [void](New-ItemProperty `
            -Path $uninstallRegistryPath `
            -Name $entry.Key `
            -Value $propertyValue `
            -PropertyType $propertyType `
            -Force)
    }

    $startMenuShortcutBackup = Move-ManagedShortcutToBackup `
        -ShortcutPath $startMenuShortcut `
        -ExecutablePath $guiExecutable
    $startMenuShortcutMutated = $true
    Install-Shortcut `
        -ShortcutPath $startMenuShortcut `
        -ExecutablePath $guiExecutable `
        -WorkingDirectory $installDirectory

    if ($createDesktopShortcut) {
        if ($null -eq $desktopShortcutPath) {
            Stop-WithMessage 'Windows did not provide a Desktop directory for this user'
        }
        $desktopShortcutBackup = Move-ManagedShortcutToBackup `
            -ShortcutPath $desktopShortcutPath `
            -ExecutablePath $guiExecutable
        $desktopShortcutMutated = $true
        Install-Shortcut `
            -ShortcutPath $desktopShortcutPath `
            -ExecutablePath $guiExecutable `
            -WorkingDirectory $installDirectory
    }
    elseif ($null -ne $desktopShortcutPath -and (Test-Path -LiteralPath $desktopShortcutPath -PathType Leaf)) {
        $desktopTarget = Get-ShortcutTarget -ShortcutPath $desktopShortcutPath
        if (-not [string]::IsNullOrWhiteSpace($desktopTarget) -and (Test-SamePath -First $desktopTarget -Second $guiExecutable)) {
            $desktopShortcutBackup = Move-ManagedShortcutToBackup `
                -ShortcutPath $desktopShortcutPath `
                -ExecutablePath $guiExecutable
            $desktopShortcutMutated = $true
        }
    }

    $installationCommitted = $true
}
catch {
    $installationError = $_
    $rollbackErrors = @()

    try {
        Restore-ManagedShortcut `
            -ShortcutPath $desktopShortcutPath `
            -ExecutablePath $guiExecutable `
            -MutationStarted $desktopShortcutMutated `
            -BackupPath $desktopShortcutBackup
    }
    catch {
        $rollbackErrors += "Desktop shortcut: $($_.Exception.Message)"
    }

    try {
        Restore-ManagedShortcut `
            -ShortcutPath $startMenuShortcut `
            -ExecutablePath $guiExecutable `
            -MutationStarted $startMenuShortcutMutated `
            -BackupPath $startMenuShortcutBackup
    }
    catch {
        $rollbackErrors += "Start menu shortcut: $($_.Exception.Message)"
    }

    try {
        Restore-UninstallRegistration `
            -RegistryPath $uninstallRegistryPath `
            -MutationStarted $uninstallRegistryMutationStarted `
            -BackupPath $uninstallRegistryBackup
    }
    catch {
        $rollbackErrors += "Apps registration: $($_.Exception.Message)"
    }

    foreach ($installedFile in $installedFiles) {
        try {
            if (Test-Path -LiteralPath $installedFile) {
                Remove-Item -LiteralPath $installedFile -Force
            }
        }
        catch {
            $rollbackErrors += "Installed file $installedFile`: $($_.Exception.Message)"
        }
    }
    foreach ($entry in $backups.GetEnumerator()) {
        try {
            if (Test-Path -LiteralPath $entry.Value) {
                Move-Item -LiteralPath $entry.Value -Destination $entry.Key -Force
            }
        }
        catch {
            $rollbackErrors += "File backup $($entry.Value)`: $($_.Exception.Message)"
        }
    }

    if ($rollbackErrors.Count -ne 0) {
        Write-Warning ("PaperlessLLM could not completely roll back the failed installation:`n- {0}" -f ($rollbackErrors -join "`n- "))
    }
    throw $installationError
}
finally {
    if (Test-Path -LiteralPath $stagingDirectory) {
        try {
            Remove-Item -LiteralPath $stagingDirectory -Recurse -Force
        }
        catch {
            Write-Warning "The temporary staging directory could not be removed: $stagingDirectory"
        }
    }
}

if ($installationCommitted) {
    $cleanupErrors = @()
    foreach ($backup in $backups.Values) {
        try {
            if (Test-Path -LiteralPath $backup) {
                Remove-Item -LiteralPath $backup -Force
            }
        }
        catch {
            $cleanupErrors += [string]$backup
        }
    }
    foreach ($backup in @($startMenuShortcutBackup, $desktopShortcutBackup)) {
        if (-not [string]::IsNullOrWhiteSpace($backup)) {
            try {
                if (Test-Path -LiteralPath $backup) {
                    Remove-Item -LiteralPath $backup -Force
                }
            }
            catch {
                $cleanupErrors += [string]$backup
            }
        }
    }
    if (-not [string]::IsNullOrWhiteSpace($uninstallRegistryBackup)) {
        try {
            if (Test-Path -LiteralPath $uninstallRegistryBackup) {
                Remove-Item -LiteralPath $uninstallRegistryBackup -Recurse -Force
            }
        }
        catch {
            $cleanupErrors += [string]$uninstallRegistryBackup
        }
    }
    if ($cleanupErrors.Count -ne 0) {
        Write-Warning ("The upgrade succeeded, but temporary rollback backups could not be removed: {0}" -f ($cleanupErrors -join ', '))
    }
}

Write-Host ''
Write-Host "PaperlessLLM $version was installed for the current user."
Write-Host "Install location: $installDirectory"
Write-Host "Start menu shortcut: $startMenuShortcut"
if ($createDesktopShortcut) {
    Write-Host "Desktop shortcut: $desktopShortcutPath"
}
Write-Host 'Your database and logs remain in the separate user data directory.'
Write-Host 'Saved credentials remain protected by Windows Credential Manager.'
