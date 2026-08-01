#requires -Version 5.1

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string] $ConsoleBinary,
    [Parameter(Mandatory = $true)][string] $GuiBinary,
    [Parameter(Mandatory = $true)][string] $Version,
    [Parameter(Mandatory = $true)]
    [ValidateSet('x86_64', 'aarch64')]
    [string] $Architecture,
    [Parameter(Mandatory = $true)][string] $OutputDirectory,
    [Parameter(Mandatory = $true)][string] $License,
    [Parameter(Mandatory = $true)][string] $DocumentationDirectory
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Stop-WithMessage {
    param([Parameter(Mandatory = $true)][string] $Message)

    throw "create-release-bundle: $Message"
}

function Get-FullPath {
    param([Parameter(Mandatory = $true)][string] $Path)

    return [System.IO.Path]::GetFullPath($Path)
}

function Assert-PortableExecutable {
    param(
        [Parameter(Mandatory = $true)][string] $Path,
        [Parameter(Mandatory = $true)][string] $Label,
        [Parameter(Mandatory = $true)][int] $ExpectedMachine,
        [Parameter(Mandatory = $true)][int] $ExpectedSubsystem
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        Stop-WithMessage "$Label not found: $Path"
    }
    $stream = [System.IO.File]::Open($Path, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::Read)
    $reader = New-Object System.IO.BinaryReader($stream)
    try {
        if ($stream.Length -lt 96 -or $reader.ReadUInt16() -ne 0x5A4D) {
            Stop-WithMessage "$Label is not a Windows PE executable: $Path"
        }
        $stream.Position = 0x3C
        $peOffset = $reader.ReadUInt32()
        if ($peOffset -gt ($stream.Length - 96)) {
            Stop-WithMessage "$Label has an invalid Windows PE header: $Path"
        }
        $stream.Position = $peOffset
        if ($reader.ReadUInt32() -ne 0x00004550) {
            Stop-WithMessage "$Label has an invalid Windows PE signature: $Path"
        }
        $machine = $reader.ReadUInt16()
        $stream.Position = $peOffset + 24
        $optionalHeaderMagic = $reader.ReadUInt16()
        if ($optionalHeaderMagic -ne 0x010B -and $optionalHeaderMagic -ne 0x020B) {
            Stop-WithMessage "$Label has an unsupported Windows optional header: $Path"
        }
        $stream.Position = $peOffset + 24 + 68
        $subsystem = $reader.ReadUInt16()
        if ($machine -ne $ExpectedMachine) {
            Stop-WithMessage ("$Label targets machine 0x{0:X4}, not the requested $Architecture machine 0x{1:X4}" -f $machine, $ExpectedMachine)
        }
        if ($subsystem -ne $ExpectedSubsystem) {
            Stop-WithMessage ("$Label uses Windows subsystem $subsystem instead of $ExpectedSubsystem")
        }
    }
    finally {
        $reader.Dispose()
    }
}

function Assert-PowerShellSyntax {
    param([Parameter(Mandatory = $true)][string] $Path)

    $tokens = $null
    $errors = $null
    [void][System.Management.Automation.Language.Parser]::ParseFile($Path, [ref]$tokens, [ref]$errors)
    if ($errors.Count -ne 0) {
        $messages = ($errors | ForEach-Object { $_.Message }) -join '; '
        Stop-WithMessage "PowerShell syntax validation failed for $Path`: $messages"
    }
}

if ($Version -notmatch '^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$') {
    Stop-WithMessage 'Version must be semantic version text without a leading v'
}

$consoleBinaryPath = Get-FullPath -Path $ConsoleBinary
$guiBinaryPath = Get-FullPath -Path $GuiBinary
$licensePath = Get-FullPath -Path $License
$documentationPath = Get-FullPath -Path $DocumentationDirectory
$outputPath = Get-FullPath -Path $OutputDirectory
$scriptDirectory = Split-Path -Parent $MyInvocation.MyCommand.Path

$expectedMachine = if ($Architecture -eq 'x86_64') { 0x8664 } else { 0xAA64 }
Assert-PortableExecutable `
    -Path $consoleBinaryPath `
    -Label 'console binary' `
    -ExpectedMachine $expectedMachine `
    -ExpectedSubsystem 3
Assert-PortableExecutable `
    -Path $guiBinaryPath `
    -Label 'GUI binary' `
    -ExpectedMachine $expectedMachine `
    -ExpectedSubsystem 2
if (-not (Test-Path -LiteralPath $licensePath -PathType Leaf)) {
    Stop-WithMessage "license file not found: $licensePath"
}
$documentationItem = Get-Item -LiteralPath $documentationPath -Force -ErrorAction SilentlyContinue
if ($null -eq $documentationItem -or -not $documentationItem.PSIsContainer) {
    Stop-WithMessage "documentation directory not found: $documentationPath"
}
if (($documentationItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
    Stop-WithMessage "documentation directory must not be a junction or symbolic link: $documentationPath"
}
$requiredDocumentation = @(
    'backup-and-restore.md',
    'native-install.md',
    'security.md',
    'release-process.md'
)
foreach ($fileName in $requiredDocumentation) {
    if (-not (Test-Path -LiteralPath (Join-Path $documentationPath $fileName) -PathType Leaf)) {
        Stop-WithMessage "required documentation file not found: $fileName"
    }
}
$documentationReparsePoints = @(
    Get-ChildItem -LiteralPath $documentationPath -Force -Recurse |
        Where-Object { ($_.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0 }
)
if ($documentationReparsePoints.Count -ne 0) {
    Stop-WithMessage 'documentation directory must not contain junctions or symbolic links'
}

$packagingFiles = @(
    'Install-PaperlessLLM.ps1',
    'Uninstall-PaperlessLLM.ps1',
    'Install PaperlessLLM.cmd',
    'Uninstall PaperlessLLM.cmd',
    'README-Windows.txt.in'
)
foreach ($fileName in $packagingFiles) {
    $path = Join-Path $scriptDirectory $fileName
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        Stop-WithMessage "packaging file not found: $path"
    }
    if ($fileName.EndsWith('.ps1', [System.StringComparison]::OrdinalIgnoreCase)) {
        Assert-PowerShellSyntax -Path $path
    }
}

[void](New-Item -ItemType Directory -Path $outputPath -Force)
$outputItem = Get-Item -LiteralPath $outputPath -Force
if (-not $outputItem.PSIsContainer) {
    Stop-WithMessage "output path is not a directory: $outputPath"
}
if (($outputItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
    Stop-WithMessage "output directory must not be a junction or symbolic link: $outputPath"
}

$archiveBaseName = "paperless-llm-$Version-$Architecture-pc-windows-msvc"
$archivePath = Join-Path $outputPath "$archiveBaseName.zip"
if (Test-Path -LiteralPath $archivePath) {
    Stop-WithMessage "output archive already exists: $archivePath"
}

$stagingContainer = Join-Path $outputPath ('.PaperlessLLM-package-{0}' -f [guid]::NewGuid().ToString('N'))
$stagingRoot = Join-Path $stagingContainer $archiveBaseName
try {
    [void](New-Item -ItemType Directory -Path $stagingRoot -Force)
    Copy-Item -LiteralPath $consoleBinaryPath -Destination (Join-Path $stagingRoot 'paperless-llm.exe')
    Copy-Item -LiteralPath $guiBinaryPath -Destination (Join-Path $stagingRoot 'PaperlessLLM.exe')
    Copy-Item -LiteralPath $licensePath -Destination (Join-Path $stagingRoot 'LICENSE')
    Copy-Item `
        -LiteralPath $documentationPath `
        -Destination (Join-Path $stagingRoot 'docs') `
        -Recurse
    foreach ($fileName in @(
        'Install-PaperlessLLM.ps1',
        'Uninstall-PaperlessLLM.ps1',
        'Install PaperlessLLM.cmd',
        'Uninstall PaperlessLLM.cmd'
    )) {
        Copy-Item -LiteralPath (Join-Path $scriptDirectory $fileName) -Destination (Join-Path $stagingRoot $fileName)
    }

    $readme = Get-Content -LiteralPath (Join-Path $scriptDirectory 'README-Windows.txt.in') -Raw
    $readme = $readme.Replace('__VERSION__', $Version).Replace('__ARCHITECTURE__', $Architecture)
    Set-Content -LiteralPath (Join-Path $stagingRoot 'README-Windows.txt') -Value $readme -Encoding UTF8
    Set-Content -LiteralPath (Join-Path $stagingRoot 'VERSION') -Value $Version -Encoding ASCII

    $checksums = foreach ($fileName in @('PaperlessLLM.exe', 'paperless-llm.exe')) {
        $hash = (Get-FileHash -LiteralPath (Join-Path $stagingRoot $fileName) -Algorithm SHA256).Hash.ToLowerInvariant()
        "$hash  $fileName"
    }
    Set-Content -LiteralPath (Join-Path $stagingRoot 'SHA256SUMS') -Value $checksums -Encoding ASCII

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [System.IO.Compression.ZipFile]::CreateFromDirectory(
        $stagingContainer,
        $archivePath,
        [System.IO.Compression.CompressionLevel]::Optimal,
        $false
    )
    if (-not (Test-Path -LiteralPath $archivePath -PathType Leaf)) {
        Stop-WithMessage 'ZIP creation did not produce the expected archive'
    }
}
catch {
    if (Test-Path -LiteralPath $archivePath) {
        Remove-Item -LiteralPath $archivePath -Force
    }
    throw
}
finally {
    if (Test-Path -LiteralPath $stagingContainer) {
        Remove-Item -LiteralPath $stagingContainer -Recurse -Force
    }
}

Write-Output $archivePath
