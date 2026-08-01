#requires -Version 5.1

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string] $Archive
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Stop-WithMessage {
    param([Parameter(Mandatory = $true)][string] $Message)

    throw "test-release-bundle: $Message"
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

function Assert-PortableExecutable {
    param(
        [Parameter(Mandatory = $true)][string] $Path,
        [Parameter(Mandatory = $true)][int] $ExpectedMachine,
        [Parameter(Mandatory = $true)][int] $ExpectedSubsystem
    )

    $stream = [System.IO.File]::Open($Path, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::Read)
    $reader = New-Object System.IO.BinaryReader($stream)
    try {
        if ($stream.Length -lt 96 -or $reader.ReadUInt16() -ne 0x5A4D) {
            Stop-WithMessage "not a Windows PE executable: $Path"
        }
        $stream.Position = 0x3C
        $peOffset = $reader.ReadUInt32()
        if ($peOffset -gt ($stream.Length - 96)) {
            Stop-WithMessage "invalid Windows PE header: $Path"
        }
        $stream.Position = $peOffset
        if ($reader.ReadUInt32() -ne 0x00004550) {
            Stop-WithMessage "invalid Windows PE signature: $Path"
        }
        $machine = $reader.ReadUInt16()
        $stream.Position = $peOffset + 24 + 68
        $subsystem = $reader.ReadUInt16()
        if ($machine -ne $ExpectedMachine) {
            Stop-WithMessage ("unexpected Windows PE machine 0x{0:X4}: $Path" -f $machine)
        }
        if ($subsystem -ne $ExpectedSubsystem) {
            Stop-WithMessage "unexpected Windows PE subsystem $subsystem`: $Path"
        }
    }
    finally {
        $reader.Dispose()
    }
}

$archivePath = [System.IO.Path]::GetFullPath($Archive)
if (-not (Test-Path -LiteralPath $archivePath -PathType Leaf)) {
    Stop-WithMessage "archive not found: $archivePath"
}
if ([System.IO.Path]::GetExtension($archivePath) -ne '.zip') {
    Stop-WithMessage 'archive path must end in .zip'
}

$temporaryDirectory = Join-Path ([System.IO.Path]::GetTempPath()) ('PaperlessLLM-bundle-test-{0}' -f [guid]::NewGuid().ToString('N'))
try {
    [void](New-Item -ItemType Directory -Path $temporaryDirectory)
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [System.IO.Compression.ZipFile]::ExtractToDirectory($archivePath, $temporaryDirectory)

    $roots = @(Get-ChildItem -LiteralPath $temporaryDirectory -Force)
    if ($roots.Count -ne 1 -or -not $roots[0].PSIsContainer) {
        Stop-WithMessage 'archive must contain exactly one top-level release directory'
    }
    $releaseRoot = $roots[0].FullName

    $expectedRootItems = @(
        'docs',
        'Install PaperlessLLM.cmd',
        'Install-PaperlessLLM.ps1',
        'LICENSE',
        'PaperlessLLM.exe',
        'README-Windows.txt',
        'SHA256SUMS',
        'Uninstall PaperlessLLM.cmd',
        'Uninstall-PaperlessLLM.ps1',
        'VERSION',
        'paperless-llm.exe'
    ) | Sort-Object
    $actualItems = @(Get-ChildItem -LiteralPath $releaseRoot -Force)
    $actualRootItems = @($actualItems | ForEach-Object { $_.Name } | Sort-Object)
    if (($actualRootItems -join "`n") -ne ($expectedRootItems -join "`n")) {
        Stop-WithMessage "archive layout differs from the expected release layout: $($actualRootItems -join ', ')"
    }

    $documentationDirectory = Join-Path $releaseRoot 'docs'
    if (-not (Test-Path -LiteralPath $documentationDirectory -PathType Container)) {
        Stop-WithMessage 'archive does not contain a docs directory'
    }
    foreach ($fileName in @(
        'backup-and-restore.md',
        'native-install.md',
        'security.md',
        'release-process.md'
    )) {
        if (-not (Test-Path -LiteralPath (Join-Path $documentationDirectory $fileName) -PathType Leaf)) {
            Stop-WithMessage "archive is missing required documentation: docs/$fileName"
        }
    }

    if ($roots[0].Name -notmatch '^paperless-llm-[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?-(x86_64|aarch64)-pc-windows-msvc$') {
        Stop-WithMessage "release directory has an unexpected name: $($roots[0].Name)"
    }
    $expectedMachine = if ($Matches[1] -eq 'x86_64') { 0x8664 } else { 0xAA64 }
    Assert-PortableExecutable `
        -Path (Join-Path $releaseRoot 'PaperlessLLM.exe') `
        -ExpectedMachine $expectedMachine `
        -ExpectedSubsystem 2
    Assert-PortableExecutable `
        -Path (Join-Path $releaseRoot 'paperless-llm.exe') `
        -ExpectedMachine $expectedMachine `
        -ExpectedSubsystem 3
    foreach ($fileName in @('Install-PaperlessLLM.ps1', 'Uninstall-PaperlessLLM.ps1')) {
        Assert-PowerShellSyntax -Path (Join-Path $releaseRoot $fileName)
    }

    $expected = @{}
    foreach ($line in @(Get-Content -LiteralPath (Join-Path $releaseRoot 'SHA256SUMS'))) {
        if ($line -notmatch '^([0-9a-f]{64})  (PaperlessLLM\.exe|paperless-llm\.exe)$') {
            Stop-WithMessage 'SHA256SUMS contains a malformed entry'
        }
        if ($expected.ContainsKey($Matches[2])) {
            Stop-WithMessage "SHA256SUMS contains a duplicate entry for $($Matches[2])"
        }
        $expected[$Matches[2]] = $Matches[1]
    }
    if ($expected.Count -ne 2) {
        Stop-WithMessage 'SHA256SUMS must contain exactly two entries'
    }
    foreach ($fileName in @('PaperlessLLM.exe', 'paperless-llm.exe')) {
        $actualHash = (Get-FileHash -LiteralPath (Join-Path $releaseRoot $fileName) -Algorithm SHA256).Hash
        if (-not [string]::Equals($actualHash, $expected[$fileName], [System.StringComparison]::OrdinalIgnoreCase)) {
            Stop-WithMessage "SHA-256 mismatch for $fileName"
        }
    }

    if ((Get-Content -LiteralPath (Join-Path $releaseRoot 'README-Windows.txt') -Raw) -match '__[A-Z]+__') {
        Stop-WithMessage 'README-Windows.txt contains an unresolved template placeholder'
    }
}
finally {
    if (Test-Path -LiteralPath $temporaryDirectory) {
        Remove-Item -LiteralPath $temporaryDirectory -Recurse -Force
    }
}

Write-Output 'Windows release bundle validation passed.'
