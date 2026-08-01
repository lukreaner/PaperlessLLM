#requires -Version 5.1

[CmdletBinding()]
param(
    [switch] $Quiet
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$ApplicationId = 'io.paperlessllm.PaperlessLLM'
$installDirectory = [System.IO.Path]::GetFullPath($PSScriptRoot).TrimEnd('\')
$markerPath = Join-Path $installDirectory '.paperlessllm-install.json'
$guiExecutable = Join-Path $installDirectory 'PaperlessLLM.exe'
$consoleExecutable = Join-Path $installDirectory 'paperless-llm.exe'

function Stop-WithMessage {
    param([Parameter(Mandatory = $true)][string] $Message)

    throw "PaperlessLLM uninstaller: $Message"
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

function Remove-ManagedShortcut {
    param([Parameter(Mandatory = $true)][string] $ShortcutPath)

    if (-not (Test-Path -LiteralPath $ShortcutPath -PathType Leaf)) {
        return
    }
    $target = Get-ShortcutTarget -ShortcutPath $ShortcutPath
    if (-not [string]::IsNullOrWhiteSpace($target) -and (Test-SamePath -First $target -Second $guiExecutable)) {
        Remove-Item -LiteralPath $ShortcutPath -Force
    }
}

if (-not (Test-Path -LiteralPath $markerPath -PathType Leaf)) {
    Stop-WithMessage 'the installation marker is missing; no files were removed'
}
try {
    $marker = Get-Content -LiteralPath $markerPath -Raw | ConvertFrom-Json
}
catch {
    Stop-WithMessage 'the installation marker is invalid; no files were removed'
}
if ($marker.applicationId -ne $ApplicationId -or [int]$marker.schema -ne 1) {
    Stop-WithMessage 'the installation marker does not belong to PaperlessLLM; no files were removed'
}

$directoryItem = Get-Item -LiteralPath $installDirectory -Force
if (($directoryItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
    Stop-WithMessage 'the install directory is a junction or symbolic link; no files were removed'
}

$normalizedExecutables = @(
    (Get-FullPath -Path $guiExecutable),
    (Get-FullPath -Path $consoleExecutable)
)
foreach ($processName in @('PaperlessLLM', 'paperless-llm')) {
    foreach ($process in @(Get-Process -Name $processName -ErrorAction SilentlyContinue)) {
        try {
            if ($null -ne $process.Path -and ($normalizedExecutables -contains (Get-FullPath -Path $process.Path))) {
                Stop-WithMessage 'PaperlessLLM is running. Use Quit PaperlessLLM in the dashboard, then uninstall again.'
            }
        }
        catch {
            if ($_.Exception.Message -like 'PaperlessLLM uninstaller:*') {
                throw
            }
            # A process that exits during inspection is already safe to ignore.
        }
    }
}

$startMenuPrograms = [Environment]::GetFolderPath([Environment+SpecialFolder]::Programs)
$desktopDirectory = [Environment]::GetFolderPath([Environment+SpecialFolder]::DesktopDirectory)
if (-not [string]::IsNullOrWhiteSpace($startMenuPrograms)) {
    Remove-ManagedShortcut -ShortcutPath (Join-Path $startMenuPrograms 'PaperlessLLM.lnk')
}
if (-not [string]::IsNullOrWhiteSpace($desktopDirectory)) {
    Remove-ManagedShortcut -ShortcutPath (Join-Path $desktopDirectory 'PaperlessLLM.lnk')
}

$uninstallRegistryPath = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\PaperlessLLM'
if (Test-Path -LiteralPath $uninstallRegistryPath) {
    Remove-Item -LiteralPath $uninstallRegistryPath -Recurse -Force
}

# Delete only files owned by the installer. Unknown files are deliberately left
# in place, and the PaperlessLLM user-data directory is never touched.
$removeNow = @(
    'PaperlessLLM.exe',
    'paperless-llm.exe',
    'LICENSE',
    'README-Windows.txt',
    'VERSION',
    'SHA256SUMS'
)
foreach ($fileName in $removeNow) {
    $path = Join-Path $installDirectory $fileName
    if (Test-Path -LiteralPath $path -PathType Leaf) {
        Remove-Item -LiteralPath $path -Force
    }
}

# This PowerShell process may still have the uninstaller open, while a
# double-clicked command wrapper remains active until this process exits. A
# short-lived hidden helper removes exactly those final managed files and then
# removes the install directory only if it is empty.
function ConvertTo-SingleQuotedLiteral {
    param([Parameter(Mandatory = $true)][string] $Value)

    return "'" + $Value.Replace("'", "''") + "'"
}

$cleanupPaths = @(
    (Join-Path $installDirectory 'Uninstall-PaperlessLLM.ps1'),
    (Join-Path $installDirectory 'Uninstall PaperlessLLM.cmd'),
    $markerPath
)
$cleanupLines = @(
    "`$ErrorActionPreference = 'SilentlyContinue'",
    "Wait-Process -Id $PID -ErrorAction SilentlyContinue",
    'Start-Sleep -Milliseconds 750'
)
foreach ($path in $cleanupPaths) {
    $cleanupLines += 'Remove-Item -LiteralPath {0} -Force -ErrorAction SilentlyContinue' -f (ConvertTo-SingleQuotedLiteral -Value $path)
}
$cleanupLines += 'Remove-Item -LiteralPath {0} -Force -ErrorAction SilentlyContinue' -f (ConvertTo-SingleQuotedLiteral -Value $installDirectory)
$cleanupScript = $cleanupLines -join "`r`n"
$encodedCleanup = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($cleanupScript))
$windowsPowerShell = Join-Path $PSHOME 'powershell.exe'
if (-not (Test-Path -LiteralPath $windowsPowerShell -PathType Leaf)) {
    $windowsPowerShell = (Get-Command 'powershell.exe' -ErrorAction Stop).Source
}
[void](Start-Process `
    -FilePath $windowsPowerShell `
    -WindowStyle Hidden `
    -ArgumentList @('-NoLogo', '-NoProfile', '-NonInteractive', '-EncodedCommand', $encodedCleanup))

if (-not $Quiet) {
    Write-Host ''
    Write-Host 'PaperlessLLM was uninstalled for the current user.'
    Write-Host 'Your database, settings, and logs were preserved.'
    Write-Host 'Saved credentials were preserved in Windows Credential Manager.'
    Write-Host 'To remove local file data too, delete this directory manually after making any desired backup:'
    Write-Host (Join-Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)) 'paperless-llm')
}
