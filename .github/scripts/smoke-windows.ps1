param(
    [Parameter(Mandatory = $true)]
    [string]$Binary,

    [Parameter(Mandatory = $true)]
    [string]$GuiBinary,

    [Parameter(Mandatory = $true)]
    [string]$Target,

    [int]$Port = 18788
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Get-PeSubsystem {
    param([Parameter(Mandatory = $true)][string]$Path)

    $stream = [IO.File]::OpenRead($Path)
    $reader = [IO.BinaryReader]::new($stream)
    try {
        if ($stream.Length -lt 96 -or $reader.ReadUInt16() -ne 0x5a4d) {
            throw "Windows executable does not have a valid DOS header: $Path"
        }

        $stream.Position = 0x3c
        $peOffset = [int64]$reader.ReadUInt32()
        $optionalHeaderOffset = $peOffset + 24
        $subsystemOffset = $optionalHeaderOffset + 68
        if ($peOffset -lt 64 -or $subsystemOffset + 2 -gt $stream.Length) {
            throw "Windows executable has an invalid PE header offset: $Path"
        }

        $stream.Position = $peOffset
        if ($reader.ReadUInt32() -ne 0x00004550) {
            throw "Windows executable does not have a valid PE signature: $Path"
        }
        $stream.Position = $optionalHeaderOffset
        $optionalHeaderMagic = $reader.ReadUInt16()
        if ($optionalHeaderMagic -ne 0x010b -and $optionalHeaderMagic -ne 0x020b) {
            throw "Windows executable has an unknown PE optional-header format: $Path"
        }

        $stream.Position = $subsystemOffset
        return $reader.ReadUInt16()
    }
    finally {
        $reader.Dispose()
    }
}

$binaryPath = (Resolve-Path -LiteralPath $Binary).Path
$guiBinaryPath = (Resolve-Path -LiteralPath $GuiBinary).Path
if ((Get-PeSubsystem -Path $binaryPath) -ne 3) {
    throw "paperless-llm.exe is not a Windows console executable: $binaryPath"
}
if ((Get-PeSubsystem -Path $guiBinaryPath) -ne 2) {
    throw "PaperlessLLM.exe is not a Windows GUI executable: $guiBinaryPath"
}

& $binaryPath --help | Out-Null
if ($LASTEXITCODE -ne 0) {
    throw "PaperlessLLM --help failed with exit code $LASTEXITCODE"
}

$smokeDir = Join-Path $env:RUNNER_TEMP "paperless-llm-smoke-$Target"
$stdoutLog = Join-Path $env:RUNNER_TEMP "paperless-llm-smoke-$Target.stdout.log"
$stderrLog = Join-Path $env:RUNNER_TEMP "paperless-llm-smoke-$Target.stderr.log"
New-Item -ItemType Directory -Path $smokeDir -Force | Out-Null

$arguments = @(
    "serve",
    "--data-dir", $smokeDir,
    "--bind", "127.0.0.1",
    "--port", $Port.ToString(),
    "--no-open"
)
$startParameters = @{
    FilePath = $binaryPath
    ArgumentList = $arguments
    RedirectStandardOutput = $stdoutLog
    RedirectStandardError = $stderrLog
    PassThru = $true
}
$server = Start-Process @startParameters

function Write-SmokeLogs {
    if (Test-Path -LiteralPath $stdoutLog) {
        Get-Content -LiteralPath $stdoutLog
    }
    if (Test-Path -LiteralPath $stderrLog) {
        Get-Content -LiteralPath $stderrLog
    }
}

try {
    $healthUri = "http://127.0.0.1:$Port/healthz"
    $healthy = $false
    for ($attempt = 0; $attempt -lt 30; $attempt++) {
        try {
            Invoke-WebRequest -UseBasicParsing -Uri $healthUri -TimeoutSec 2 | Out-Null
            $healthy = $true
            break
        }
        catch {
            if ($server.HasExited) {
                Write-SmokeLogs
                throw "PaperlessLLM exited before becoming healthy"
            }
            Start-Sleep -Seconds 1
        }
    }

    if (-not $healthy) {
        Write-SmokeLogs
        throw "PaperlessLLM did not become healthy within 30 seconds"
    }

    $launchArguments = @(
        "launch",
        "--data-dir", $smokeDir,
        "--bind", "127.0.0.1",
        "--port", $Port.ToString(),
        "--no-open"
    )
    & $binaryPath @launchArguments
    if ($LASTEXITCODE -ne 0) {
        throw "PaperlessLLM launch probe failed with exit code $LASTEXITCODE"
    }

    $shutdownParameters = @{
        Method = "Post"
        Uri = "http://127.0.0.1:$Port/api/shutdown"
        Headers = @{ "x-paperlessllm-action" = "shutdown" }
        TimeoutSec = 5
    }
    Invoke-RestMethod @shutdownParameters | Out-Null
    if (-not $server.WaitForExit(10000)) {
        throw "PaperlessLLM did not exit after its shutdown request"
    }
    if ($server.ExitCode -ne 0) {
        Write-SmokeLogs
        throw "PaperlessLLM exited with code $($server.ExitCode)"
    }
}
finally {
    if (-not $server.HasExited) {
        Stop-Process -Id $server.Id -Force
        $server.WaitForExit()
    }
}
