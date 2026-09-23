<#
.SYNOPSIS
  Admin-side, run once: downloads the models Noetis needs into a staging folder with the
  same layout as the app's models folder, verifying every file's size and SHA-256.
  Package the folder with the installer and deploy it with deploy-policy.ps1 -ModelsSource.

.EXAMPLE
  .\fetch-models.ps1 -Destination D:\noetis-models
#>
param([Parameter(Mandatory)] [string] $Destination)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

# Pinned commits and hashes; must match frontend/src-tauri/src (parakeet_engine.rs, diarization.rs).
$parakeet = 'https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/8f23f0c03c8761650bdb5b40aaf3e40d2c15f1ce'
$files = @(
    @{ Url = "$parakeet/encoder-model.int8.onnx";       Path = 'parakeet\parakeet-tdt-0.6b-v3-int8\encoder-model.int8.onnx';       Size = 652183999; Sha = '6139d2fa7e1b086097b277c7149725edbab89cc7c7ae64b23c741be4055aff09' },
    @{ Url = "$parakeet/decoder_joint-model.int8.onnx"; Path = 'parakeet\parakeet-tdt-0.6b-v3-int8\decoder_joint-model.int8.onnx'; Size = 18202004;  Sha = 'eea7483ee3d1a30375daedc8ed83e3960c91b098812127a0d99d1c8977667a70' },
    @{ Url = "$parakeet/nemo128.onnx";                  Path = 'parakeet\parakeet-tdt-0.6b-v3-int8\nemo128.onnx';                  Size = 139764;    Sha = 'a9fde1486ebfcc08f328d75ad4610c67835fea58c73ba57e3209a6f6cf019e9f' },
    @{ Url = "$parakeet/vocab.txt";                     Path = 'parakeet\parakeet-tdt-0.6b-v3-int8\vocab.txt';                     Size = 93939;     Sha = 'd58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d' },
    @{ Url = 'https://huggingface.co/Wespeaker/wespeaker-voxceleb-resnet34/resolve/ff1ac5bca8ef11e90662b879aa923979e0bd277b/voxceleb_resnet34.onnx'
       Path = 'diarization\voxceleb_resnet34.onnx'; Size = 26534127; Sha = '9fea6516d7ad6bf0a76c7689f5a49b65d330fad6dde96c91bb4435ffbfe056a1' }
)

foreach ($f in $files) {
    $target = Join-Path $Destination $f.Path
    New-Item -ItemType Directory -Force -Path (Split-Path $target) | Out-Null
    if (-not (Test-Path $target) -or (Get-Item $target).Length -ne $f.Size) {
        Write-Output "Downloading $($f.Path)..."
        Invoke-WebRequest -Uri $f.Url -OutFile $target -UseBasicParsing
    }
    $sha = (Get-FileHash -Algorithm SHA256 $target).Hash.ToLower()
    if ((Get-Item $target).Length -ne $f.Size -or $sha -ne $f.Sha) {
        Remove-Item $target -Force
        throw "Verification failed for $($f.Path) (sha256 $sha)"
    }
    Write-Output "OK  $($f.Path)"
}
Write-Output "Models staged in $Destination"
