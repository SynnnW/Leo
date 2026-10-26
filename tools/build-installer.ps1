# Bangun Leo lalu installernya. Hasil: dist\LeoSetup.exe
# Jalankan dari mana saja: powershell -ExecutionPolicy Bypass -File tools\build-installer.ps1
#
# Opsional: -TargetDir <folder> untuk membangun di folder target lain
# (berguna bila target\release\leo.exe sedang berjalan dan terkunci).
param([string]$TargetDir = "")

# Cargo menulis progres ke stderr; jangan anggap itu error, cek $LASTEXITCODE saja.
$ErrorActionPreference = "Continue"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"

$targetArgs = @()
$target = Join-Path $root "target"
if ($TargetDir) {
    # Build script installer berjalan dari folder installer\, jadi path harus absolut.
    if (-not [System.IO.Path]::IsPathRooted($TargetDir)) { $TargetDir = Join-Path $root $TargetDir }
    $targetArgs = @("--target-dir", $TargetDir)
    $target = $TargetDir
}

cargo build --release @targetArgs
if ($LASTEXITCODE -ne 0) { throw "Build Leo gagal (tutup Leo dulu bila exe terkunci)" }

$env:LEO_EXE = Join-Path $target "release\leo.exe"
cargo build --release -p leo-setup @targetArgs
if ($LASTEXITCODE -ne 0) { throw "Build installer gagal" }
Remove-Item Env:LEO_EXE

New-Item -ItemType Directory -Force (Join-Path $root "dist") | Out-Null
$out = Join-Path $root "dist\LeoSetup.exe"
Copy-Item (Join-Path $target "release\LeoSetup.exe") $out -Force
$size = (Get-Item $out).Length / 1KB
"Selesai: $out ({0:N0} KB)" -f $size
"SHA-256: " + (Get-FileHash $out -Algorithm SHA256).Hash.ToLower()
