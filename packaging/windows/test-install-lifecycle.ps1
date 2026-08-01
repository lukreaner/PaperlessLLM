#requires -Version 5.1

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string] $Archive
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Stop-WithMessage {
    param([Parameter(Mandatory = $true)][string] $Message)

    throw "test-install-lifecycle: $Message"
}

function Get-FullPath {
    param([Parameter(Mandatory = $true)][string] $Path)

    return [System.IO.Path]::GetFullPath($Path).TrimEnd('\')
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

function Get-ShortcutDetails {
    param([Parameter(Mandatory = $true)][string] $ShortcutPath)

    $shell = New-Object -ComObject WScript.Shell
    $shortcut = $null
    try {
        $shortcut = $shell.CreateShortcut($ShortcutPath)
        return [pscustomobject]@{
            TargetPath = $shortcut.TargetPath
            Arguments = $shortcut.Arguments
        }
    }
    finally {
        if ($null -ne $shortcut) {
            [void][System.Runtime.InteropServices.Marshal]::FinalReleaseComObject($shortcut)
        }
        [void][System.Runtime.InteropServices.Marshal]::FinalReleaseComObject($shell)
    }
}

function Invoke-UninstallerChild {
    param([Parameter(Mandatory = $true)][string] $Uninstaller)

    # The uninstaller starts a short-lived cleanup helper that waits for its
    # own PowerShell process to exit. Run it in a child rather than dot-sourcing
    # it into this test process, otherwise that helper would wait on the CI
    # process while the CI process waits for the helper.
    $windowsPowerShell = (Get-Command 'powershell.exe' -ErrorAction Stop).Source
    & $windowsPowerShell `
        -NoLogo `
        -NoProfile `
        -NonInteractive `
        -ExecutionPolicy Bypass `
        -File $Uninstaller `
        -Quiet
    if ($LASTEXITCODE -ne 0) {
        Stop-WithMessage "uninstaller child exited with code $LASTEXITCODE"
    }
}

function Invoke-InstallerChild {
    param(
        [Parameter(Mandatory = $true)][string] $Installer,
        [string[]] $InstallerArguments = @(),
        [switch] $ExpectFailure
    )

    $windowsPowerShell = (Get-Command 'powershell.exe' -ErrorAction Stop).Source
    $previousErrorActionPreference = $ErrorActionPreference
    try {
        # A deliberately rejected install writes a native-process error record.
        # Keep that output visible without letting the parent test stop before
        # it can assert the child's exit code.
        $ErrorActionPreference = 'Continue'
        & $windowsPowerShell `
            -NoLogo `
            -NoProfile `
            -NonInteractive `
            -ExecutionPolicy Bypass `
            -File $Installer `
            @InstallerArguments
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }

    if ($ExpectFailure) {
        if ($exitCode -eq 0) {
            Stop-WithMessage 'installer unexpectedly succeeded during a rollback test'
        }
    }
    elseif ($exitCode -ne 0) {
        Stop-WithMessage "installer child exited with code $exitCode"
    }
}

function Set-TestShortcut {
    param(
        [Parameter(Mandatory = $true)][string] $ShortcutPath,
        [Parameter(Mandatory = $true)][string] $TargetPath,
        [Parameter(Mandatory = $true)][string] $WorkingDirectory,
        [Parameter(Mandatory = $true)][string] $Description
    )

    [void](New-Item -ItemType Directory -Path (Split-Path -Parent $ShortcutPath) -Force)
    $shell = New-Object -ComObject WScript.Shell
    $shortcut = $null
    try {
        $shortcut = $shell.CreateShortcut($ShortcutPath)
        $shortcut.TargetPath = $TargetPath
        $shortcut.Arguments = ''
        $shortcut.WorkingDirectory = $WorkingDirectory
        $shortcut.Description = $Description
        $shortcut.Save()
    }
    finally {
        if ($null -ne $shortcut) {
            [void][System.Runtime.InteropServices.Marshal]::FinalReleaseComObject($shortcut)
        }
        [void][System.Runtime.InteropServices.Marshal]::FinalReleaseComObject($shell)
    }
}

function Get-UninstallRegistrationSnapshot {
    $relativePath = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\PaperlessLLM'
    $registryKey = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($relativePath)
    if ($null -eq $registryKey) {
        Stop-WithMessage 'cannot snapshot the missing Apps registration'
    }

    try {
        $values = @()
        foreach ($name in @($registryKey.GetValueNames() | Sort-Object)) {
            $value = $registryKey.GetValue(
                $name,
                $null,
                [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames
            )
            $normalizedValue = if ($value -is [byte[]]) {
                [Convert]::ToBase64String($value)
            }
            elseif ($value -is [string[]]) {
                @($value)
            }
            else {
                $value
            }
            $values += [ordered]@{
                name = $name
                kind = [string]($registryKey.GetValueKind($name))
                value = $normalizedValue
            }
        }
        $snapshot = [ordered]@{
            values = $values
            subkeys = @($registryKey.GetSubKeyNames() | Sort-Object)
        }
        return ($snapshot | ConvertTo-Json -Depth 6 -Compress)
    }
    finally {
        $registryKey.Dispose()
    }
}

function Assert-NoRollbackBackups {
    param(
        [Parameter(Mandatory = $true)][string] $InstallDirectory,
        [Parameter(Mandatory = $true)][string] $StartMenuShortcut,
        [Parameter(Mandatory = $true)][string] $DesktopShortcut,
        [Parameter(Mandatory = $true)][string] $UninstallRegistryPath
    )

    $leftovers = @()
    if (Test-Path -LiteralPath $InstallDirectory) {
        $leftovers += @(Get-ChildItem -LiteralPath $InstallDirectory -Force | Where-Object { $_.Name -like '*.backup-*' })
    }
    foreach ($shortcutPath in @($StartMenuShortcut, $DesktopShortcut)) {
        $shortcutDirectory = Split-Path -Parent $shortcutPath
        if (Test-Path -LiteralPath $shortcutDirectory -PathType Container) {
            $leftovers += @(Get-ChildItem -LiteralPath $shortcutDirectory -Force | Where-Object { $_.Name -like '.PaperlessLLM.lnk.rollback-*' })
        }
    }
    $registryParent = Split-Path -Parent $UninstallRegistryPath
    if (Test-Path -LiteralPath $registryParent) {
        $leftovers += @(Get-ChildItem -LiteralPath $registryParent | Where-Object { $_.PSChildName -like '.PaperlessLLM.rollback-*' })
    }
    if ($leftovers.Count -ne 0) {
        Stop-WithMessage "installer left rollback backups behind: $($leftovers.Name -join ', ')"
    }
}

$archivePath = [System.IO.Path]::GetFullPath($Archive)
if (-not (Test-Path -LiteralPath $archivePath -PathType Leaf)) {
    Stop-WithMessage "archive not found: $archivePath"
}

$localApplicationData = [Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)
$startMenuPrograms = [Environment]::GetFolderPath([Environment+SpecialFolder]::Programs)
$desktopDirectory = [Environment]::GetFolderPath([Environment+SpecialFolder]::DesktopDirectory)
if ([string]::IsNullOrWhiteSpace($localApplicationData) -or [string]::IsNullOrWhiteSpace($startMenuPrograms)) {
    Stop-WithMessage 'Windows did not provide the current user application directories'
}

$installDirectory = Join-Path $localApplicationData 'Programs\PaperlessLLM'
$startMenuShortcut = Join-Path $startMenuPrograms 'PaperlessLLM.lnk'
$desktopShortcut = if ([string]::IsNullOrWhiteSpace($desktopDirectory)) {
    $null
}
else {
    Join-Path $desktopDirectory 'PaperlessLLM.lnk'
}
if ($null -eq $desktopShortcut) {
    Stop-WithMessage 'Windows did not provide a Desktop directory required for the rollback test'
}
$uninstallRegistryPath = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\PaperlessLLM'

# Never overwrite or remove state left on a persistent/self-hosted runner. The
# GitHub-hosted Windows job is expected to begin without a PaperlessLLM install.
foreach ($reservedPath in @($installDirectory, $startMenuShortcut, $uninstallRegistryPath)) {
    if (Test-Path -LiteralPath $reservedPath) {
        Stop-WithMessage "refusing to replace pre-existing runner state: $reservedPath"
    }
}
if ($null -ne $desktopShortcut -and (Test-Path -LiteralPath $desktopShortcut)) {
    Stop-WithMessage "refusing to replace a pre-existing Desktop shortcut: $desktopShortcut"
}

$temporaryParent = if ([string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) {
    [System.IO.Path]::GetTempPath()
}
else {
    $env:RUNNER_TEMP
}
$extractionDirectory = Join-Path $temporaryParent ('PaperlessLLM-install-test-{0}' -f [guid]::NewGuid().ToString('N'))
$dataDirectory = Join-Path $localApplicationData 'paperless-llm'
$dataDirectoryExisted = Test-Path -LiteralPath $dataDirectory
$sentinelPath = Join-Path $dataDirectory ('ci-uninstall-preservation-{0}.txt' -f [guid]::NewGuid().ToString('N'))
$sentinelValue = 'PaperlessLLM uninstall must preserve user data.'
$installAttempted = $false
$foreignDesktopShortcutCreated = $false
$foreignDesktopTarget = Join-Path $env:SystemRoot 'System32\notepad.exe'

try {
    [void](New-Item -ItemType Directory -Path $extractionDirectory)
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [System.IO.Compression.ZipFile]::ExtractToDirectory($archivePath, $extractionDirectory)

    $roots = @(Get-ChildItem -LiteralPath $extractionDirectory -Force)
    if ($roots.Count -ne 1 -or -not $roots[0].PSIsContainer) {
        Stop-WithMessage 'archive must contain exactly one top-level release directory'
    }
    $releaseDirectory = $roots[0].FullName
    $installer = Join-Path $releaseDirectory 'Install-PaperlessLLM.ps1'
    if (-not (Test-Path -LiteralPath $installer -PathType Leaf)) {
        Stop-WithMessage 'archive does not contain Install-PaperlessLLM.ps1'
    }

    [void](New-Item -ItemType Directory -Path $dataDirectory -Force)
    Set-Content -LiteralPath $sentinelPath -Value $sentinelValue -Encoding ASCII

    # Force a late first-install failure with a foreign Desktop shortcut. By
    # this point the installer has already staged application files, created
    # its Apps registration, and created its Start shortcut, so the rejection
    # exercises rollback of every newly created layer.
    Set-TestShortcut `
        -ShortcutPath $desktopShortcut `
        -TargetPath $foreignDesktopTarget `
        -WorkingDirectory (Split-Path -Parent $foreignDesktopTarget) `
        -Description 'Foreign shortcut used by the PaperlessLLM rollback test'
    $foreignDesktopShortcutCreated = $true
    $foreignDesktopHash = (Get-FileHash -LiteralPath $desktopShortcut -Algorithm SHA256).Hash

    $installAttempted = $true
    Invoke-InstallerChild `
        -Installer $installer `
        -InstallerArguments @('-DesktopShortcut') `
        -ExpectFailure

    if (Test-Path -LiteralPath $startMenuShortcut) {
        Stop-WithMessage 'failed first install left a Start menu shortcut behind'
    }
    if (Test-Path -LiteralPath $uninstallRegistryPath) {
        Stop-WithMessage 'failed first install left an Apps registration behind'
    }
    if (Test-Path -LiteralPath $installDirectory) {
        $failedInstallItems = @(Get-ChildItem -LiteralPath $installDirectory -Force)
        if ($failedInstallItems.Count -ne 0) {
            Stop-WithMessage "failed first install left application files behind: $($failedInstallItems.Name -join ', ')"
        }
    }
    if (-not (Test-Path -LiteralPath $desktopShortcut -PathType Leaf)) {
        Stop-WithMessage 'failed first install removed the foreign Desktop shortcut'
    }
    if ((Get-FileHash -LiteralPath $desktopShortcut -Algorithm SHA256).Hash -ne $foreignDesktopHash) {
        Stop-WithMessage 'failed first install modified the foreign Desktop shortcut'
    }
    $foreignShortcut = Get-ShortcutDetails -ShortcutPath $desktopShortcut
    if (-not (Test-SamePath -First $foreignShortcut.TargetPath -Second $foreignDesktopTarget)) {
        Stop-WithMessage 'failed first install changed the foreign Desktop shortcut target'
    }
    if (-not (Test-Path -LiteralPath $sentinelPath -PathType Leaf)) {
        Stop-WithMessage 'failed first install deleted the user-data sentinel'
    }
    Assert-NoRollbackBackups `
        -InstallDirectory $installDirectory `
        -StartMenuShortcut $startMenuShortcut `
        -DesktopShortcut $desktopShortcut `
        -UninstallRegistryPath $uninstallRegistryPath

    Remove-Item -LiteralPath $desktopShortcut -Force
    $foreignDesktopShortcutCreated = $false

    Invoke-InstallerChild `
        -Installer $installer `
        -InstallerArguments @('-NoDesktopShortcut')

    $installedGui = Join-Path $installDirectory 'PaperlessLLM.exe'
    $installedConsole = Join-Path $installDirectory 'paperless-llm.exe'
    foreach ($installedBinary in @($installedGui, $installedConsole)) {
        if (-not (Test-Path -LiteralPath $installedBinary -PathType Leaf)) {
            Stop-WithMessage "installer did not create $installedBinary"
        }
    }
    foreach ($fileName in @('PaperlessLLM.exe', 'paperless-llm.exe')) {
        $releaseHash = (Get-FileHash -LiteralPath (Join-Path $releaseDirectory $fileName) -Algorithm SHA256).Hash
        $installedHash = (Get-FileHash -LiteralPath (Join-Path $installDirectory $fileName) -Algorithm SHA256).Hash
        if (-not [string]::Equals($releaseHash, $installedHash, [System.StringComparison]::OrdinalIgnoreCase)) {
            Stop-WithMessage "installed $fileName differs from the release binary"
        }
    }

    if (-not (Test-Path -LiteralPath $startMenuShortcut -PathType Leaf)) {
        Stop-WithMessage 'installer did not create the Start menu shortcut'
    }
    $shortcut = Get-ShortcutDetails -ShortcutPath $startMenuShortcut
    if ([string]::IsNullOrWhiteSpace($shortcut.TargetPath) -or -not (Test-SamePath -First $shortcut.TargetPath -Second $installedGui)) {
        Stop-WithMessage 'Start menu shortcut does not target PaperlessLLM.exe directly'
    }
    if (-not [string]::IsNullOrWhiteSpace($shortcut.Arguments)) {
        Stop-WithMessage 'Start menu shortcut must launch PaperlessLLM.exe without arguments'
    }
    if ($null -ne $desktopShortcut -and (Test-Path -LiteralPath $desktopShortcut)) {
        Stop-WithMessage '-NoDesktopShortcut unexpectedly created a Desktop shortcut'
    }

    if (-not (Test-Path -LiteralPath $uninstallRegistryPath)) {
        Stop-WithMessage 'installer did not register PaperlessLLM under the current user Apps list'
    }
    $registration = Get-ItemProperty -LiteralPath $uninstallRegistryPath
    $releaseVersion = (Get-Content -LiteralPath (Join-Path $releaseDirectory 'VERSION') -Raw).Trim()
    if ($registration.DisplayName -ne 'PaperlessLLM' -or $registration.DisplayVersion -ne $releaseVersion) {
        Stop-WithMessage 'Apps registration contains incorrect display metadata'
    }
    if (-not (Test-SamePath -First $registration.InstallLocation -Second $installDirectory)) {
        Stop-WithMessage 'Apps registration contains the wrong install location'
    }
    if ($registration.UninstallString -notlike '*Uninstall-PaperlessLLM.ps1*') {
        Stop-WithMessage 'Apps registration does not contain the PaperlessLLM uninstaller'
    }

    # Make the installed state observably different from the release, then
    # trigger the same late failure during an upgrade. A correct transaction
    # must restore the exact prior file, shortcut, and registry state.
    $installedReadme = Join-Path $installDirectory 'README-Windows.txt'
    $rollbackSentinel = "upgrade rollback sentinel $([guid]::NewGuid().ToString('N'))"
    Set-Content -LiteralPath $installedReadme -Value $rollbackSentinel -Encoding ASCII
    $readmeBeforeUpgrade = (Get-FileHash -LiteralPath $installedReadme -Algorithm SHA256).Hash

    Set-TestShortcut `
        -ShortcutPath $startMenuShortcut `
        -TargetPath $installedGui `
        -WorkingDirectory $installDirectory `
        -Description $rollbackSentinel
    $startShortcutBeforeUpgrade = (Get-FileHash -LiteralPath $startMenuShortcut -Algorithm SHA256).Hash

    [void](New-ItemProperty `
        -Path $uninstallRegistryPath `
        -Name 'RollbackSentinel' `
        -Value $rollbackSentinel `
        -PropertyType String `
        -Force)
    $registrationBeforeUpgrade = Get-UninstallRegistrationSnapshot

    Set-TestShortcut `
        -ShortcutPath $desktopShortcut `
        -TargetPath $foreignDesktopTarget `
        -WorkingDirectory (Split-Path -Parent $foreignDesktopTarget) `
        -Description 'Foreign shortcut used by the PaperlessLLM upgrade rollback test'
    $foreignDesktopShortcutCreated = $true
    $foreignDesktopHash = (Get-FileHash -LiteralPath $desktopShortcut -Algorithm SHA256).Hash

    Invoke-InstallerChild `
        -Installer $installer `
        -InstallerArguments @('-DesktopShortcut') `
        -ExpectFailure

    if ((Get-FileHash -LiteralPath $installedReadme -Algorithm SHA256).Hash -ne $readmeBeforeUpgrade) {
        Stop-WithMessage 'failed upgrade did not restore the previous application file'
    }
    if ((Get-FileHash -LiteralPath $startMenuShortcut -Algorithm SHA256).Hash -ne $startShortcutBeforeUpgrade) {
        Stop-WithMessage 'failed upgrade did not restore the previous Start menu shortcut exactly'
    }
    if ((Get-UninstallRegistrationSnapshot) -ne $registrationBeforeUpgrade) {
        Stop-WithMessage 'failed upgrade did not restore the previous Apps registration exactly'
    }
    if ((Get-FileHash -LiteralPath $desktopShortcut -Algorithm SHA256).Hash -ne $foreignDesktopHash) {
        Stop-WithMessage 'failed upgrade modified the foreign Desktop shortcut'
    }
    $foreignShortcut = Get-ShortcutDetails -ShortcutPath $desktopShortcut
    if (-not (Test-SamePath -First $foreignShortcut.TargetPath -Second $foreignDesktopTarget)) {
        Stop-WithMessage 'failed upgrade changed the foreign Desktop shortcut target'
    }
    Assert-NoRollbackBackups `
        -InstallDirectory $installDirectory `
        -StartMenuShortcut $startMenuShortcut `
        -DesktopShortcut $desktopShortcut `
        -UninstallRegistryPath $uninstallRegistryPath

    Remove-Item -LiteralPath $desktopShortcut -Force
    $foreignDesktopShortcutCreated = $false

    # Complete a real upgrade too, so the post-commit path proves that its
    # temporary file, shortcut, and registry backups are all removed.
    Invoke-InstallerChild `
        -Installer $installer `
        -InstallerArguments @('-NoDesktopShortcut')
    Assert-NoRollbackBackups `
        -InstallDirectory $installDirectory `
        -StartMenuShortcut $startMenuShortcut `
        -DesktopShortcut $desktopShortcut `
        -UninstallRegistryPath $uninstallRegistryPath
    if ((Get-FileHash -LiteralPath $installedReadme -Algorithm SHA256).Hash -ne `
        (Get-FileHash -LiteralPath (Join-Path $releaseDirectory 'README-Windows.txt') -Algorithm SHA256).Hash) {
        Stop-WithMessage 'successful upgrade did not install the release application file'
    }
    if ($null -ne (Get-ItemProperty -LiteralPath $uninstallRegistryPath -Name 'RollbackSentinel' -ErrorAction SilentlyContinue)) {
        Stop-WithMessage 'successful upgrade retained a superseded Apps registration value'
    }

    $uninstaller = Join-Path $installDirectory 'Uninstall-PaperlessLLM.ps1'
    if (-not (Test-Path -LiteralPath $uninstaller -PathType Leaf)) {
        Stop-WithMessage 'installed uninstaller is missing'
    }
    Invoke-UninstallerChild -Uninstaller $uninstaller

    $cleanupFinished = $false
    foreach ($attempt in 1..100) {
        $managedFilesRemain = `
            (Test-Path -LiteralPath $installedGui) -or `
            (Test-Path -LiteralPath $installedConsole) -or `
            (Test-Path -LiteralPath (Join-Path $installDirectory '.paperlessllm-install.json'))
        if (-not $managedFilesRemain -and `
            -not (Test-Path -LiteralPath $startMenuShortcut) -and `
            -not (Test-Path -LiteralPath $uninstallRegistryPath)) {
            $cleanupFinished = $true
            break
        }
        Start-Sleep -Milliseconds 100
    }
    if (-not $cleanupFinished) {
        Stop-WithMessage 'uninstaller did not remove all managed application state within ten seconds'
    }
    if (Test-Path -LiteralPath $installDirectory) {
        $remainingInstallItems = @(Get-ChildItem -LiteralPath $installDirectory -Force)
        if ($remainingInstallItems.Count -ne 0) {
            Stop-WithMessage "uninstaller left managed files behind: $($remainingInstallItems.Name -join ', ')"
        }
    }

    if (-not (Test-Path -LiteralPath $sentinelPath -PathType Leaf)) {
        Stop-WithMessage 'uninstaller deleted the user-data sentinel'
    }
    if ((Get-Content -LiteralPath $sentinelPath -Raw).Trim() -ne $sentinelValue) {
        Stop-WithMessage 'uninstaller modified the user-data sentinel'
    }
}
finally {
    if ($foreignDesktopShortcutCreated -and (Test-Path -LiteralPath $desktopShortcut -PathType Leaf)) {
        try {
            $foreignShortcut = Get-ShortcutDetails -ShortcutPath $desktopShortcut
            if (-not [string]::IsNullOrWhiteSpace($foreignShortcut.TargetPath) -and `
                (Test-SamePath -First $foreignShortcut.TargetPath -Second $foreignDesktopTarget)) {
                Remove-Item -LiteralPath $desktopShortcut -Force
            }
            else {
                Write-Warning 'PaperlessLLM lifecycle test left a Desktop shortcut untouched because it no longer matches the test shortcut.'
            }
        }
        catch {
            Write-Warning 'PaperlessLLM lifecycle test could not safely inspect or remove its Desktop test shortcut.'
        }
    }

    # On an assertion failure, make one best-effort safe cleanup attempt using
    # the installed, marker-validating uninstaller. Never recursively delete
    # the install directory from this test.
    $installedUninstaller = Join-Path $installDirectory 'Uninstall-PaperlessLLM.ps1'
    if ($installAttempted -and (Test-Path -LiteralPath $installedUninstaller -PathType Leaf)) {
        try {
            Invoke-UninstallerChild -Uninstaller $installedUninstaller
        }
        catch {
            Write-Warning 'PaperlessLLM test installation cleanup did not complete.'
        }
    }
    if (Test-Path -LiteralPath $sentinelPath -PathType Leaf) {
        Remove-Item -LiteralPath $sentinelPath -Force
    }
    if (-not $dataDirectoryExisted -and (Test-Path -LiteralPath $dataDirectory)) {
        $remainingDataItems = @(Get-ChildItem -LiteralPath $dataDirectory -Force)
        if ($remainingDataItems.Count -eq 0) {
            Remove-Item -LiteralPath $dataDirectory -Force
        }
    }
    if (Test-Path -LiteralPath $extractionDirectory) {
        Remove-Item -LiteralPath $extractionDirectory -Recurse -Force
    }
}

Write-Output 'Windows per-user install and uninstall lifecycle validation passed.'
