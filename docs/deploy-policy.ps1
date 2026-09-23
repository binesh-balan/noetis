<#
.SYNOPSIS
  Deploys the Noetis managed policy on this PC: org AI summaries (Azure AI Foundry),
  pinned transcription model/language, IT-supplied models, no model downloads, no analytics.

.DESCRIPTION
  Run as SYSTEM or an administrator (e.g. an Intune Win32 app / platform script, 64-bit).
  - The summary API key is encrypted with machine-scope DPAPI on this device, so
    policy.json never contains it in clear text and a copied file is useless elsewhere.
  - -ModelsSource (a folder staged with fetch-models.ps1) is copied to
    %ProgramData%\Noetis\models, which the app uses instead of downloading.
  - The folder is writable by administrators only; users can read it.

.EXAMPLE
  .\deploy-policy.ps1 -Endpoint "https://contoso-ai.services.ai.azure.com/openai/v1" `
      -Model "DeepSeek-V4-Flash-0731" -ApiKey "<foundry key>" `
      -ModelsSource "\\fileserver\noetis-models" -Language en
#>
param(
    # AI summaries (all three together, or omit all three)
    [string] $Endpoint,
    [string] $Model,
    [string] $ApiKey,
    [int] $MaxTokens = 8192,

    # Transcription
    [ValidateSet('parakeet', 'localWhisper')] [string] $TranscriptionProvider = 'parakeet',
    [string] $TranscriptionModel = 'parakeet-tdt-0.6b-v3-int8',
    [string] $Language,               # e.g. "en"; omit for automatic detection

    # Models and lock-downs
    [string] $ModelsSource,           # staged folder from fetch-models.ps1
    [switch] $AllowModelDownloads,    # default: downloads blocked
    [switch] $AllowAnalytics,         # default: analytics off
    [string] $TemplatesDir
)
$ErrorActionPreference = 'Stop'

$dir = Join-Path $env:ProgramData 'Noetis'
$modelsDir = Join-Path $dir 'models'
New-Item -ItemType Directory -Force -Path $modelsDir | Out-Null

if ($ModelsSource) {
    robocopy $ModelsSource $modelsDir /E /NFL /NDL /NJH /NJS /NP | Out-Null
    if ($LASTEXITCODE -ge 8) { throw "Copying models from $ModelsSource failed (robocopy $LASTEXITCODE)" }
}
$needed = if ($TranscriptionProvider -eq 'parakeet') { "parakeet\$TranscriptionModel" } else { "ggml-$TranscriptionModel.bin" }
if (-not $AllowModelDownloads -and -not (Test-Path (Join-Path $modelsDir $needed))) {
    Write-Warning "Transcription model '$needed' is not in $modelsDir and downloads are blocked; transcription will not work until it is supplied."
}

$policy = [ordered]@{
    transcription       = [ordered]@{ provider = $TranscriptionProvider; model = $TranscriptionModel }
    modelsDir           = $modelsDir
    allowModelDownloads = [bool]$AllowModelDownloads
    disableAnalytics    = -not $AllowAnalytics
}
if ($Language) { $policy.transcription.language = $Language }
if ($TemplatesDir) { $policy.templatesDir = $TemplatesDir }

if ($Endpoint -or $Model -or $ApiKey) {
    if (-not ($Endpoint -and $Model -and $ApiKey)) { throw 'Pass -Endpoint, -Model and -ApiKey together.' }
    Add-Type -AssemblyName System.Security
    $cipher = [Security.Cryptography.ProtectedData]::Protect(
        [Text.Encoding]::UTF8.GetBytes($ApiKey), $null,
        [Security.Cryptography.DataProtectionScope]::LocalMachine)
    $policy.summary = [ordered]@{
        endpoint  = $Endpoint.TrimEnd('/')
        model     = $Model
        apiKey    = 'dpapi:v1:' + (-join ($cipher | ForEach-Object { $_.ToString('x2') }))
        maxTokens = $MaxTokens
    }
}

$file = Join-Path $dir 'policy.json'
$policy | ConvertTo-Json -Depth 4 | Set-Content -Path $file -Encoding UTF8

# Administrators/SYSTEM: full control. Users: read (the app runs as the user and must read it).
icacls $dir /inheritance:r /grant:r "*S-1-5-32-544:(OI)(CI)F" "*S-1-5-18:(OI)(CI)F" "*S-1-5-32-545:(OI)(CI)RX" /T | Out-Null

Write-Output "Noetis policy written to $file. Restart Noetis to apply."
