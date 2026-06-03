# Build Varda Windows portable ZIP
# Usage: .\scripts\ci\build-windows.ps1 [-SkipBuild]
#
# Expects:
#   - cargo build --release already done (or omit -SkipBuild)
#   - vcpkg FFmpeg installed at C:\vcpkg\installed\x64-windows
#   - NDI Runtime installed (optional)
#
# Produces: Varda-Windows-x64.zip in the project root

param(
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

$ProjectRoot = Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path))
Set-Location $ProjectRoot

Write-Host "==> Project root: $ProjectRoot"

# --- Build release binary ---
if (-not $SkipBuild) {
    Write-Host "==> Building release binary..."
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
}

$StageDir = "$ProjectRoot\Varda-Windows-x64"
if (Test-Path $StageDir) { Remove-Item -Recurse -Force $StageDir }
New-Item -ItemType Directory -Path $StageDir | Out-Null

# --- Copy binary ---
Write-Host "==> Copying varda.exe..."
Copy-Item "target\release\varda.exe" "$StageDir\varda.exe"

# --- Copy shaders ---
Write-Host "==> Copying shaders..."
Copy-Item -Recurse "shaders" "$StageDir\shaders"

# --- Copy FFmpeg DLLs from vcpkg ---
$VcpkgBin = "C:\vcpkg\installed\x64-windows\bin"
Write-Host "==> Copying FFmpeg DLLs from $VcpkgBin..."
$FfmpegDlls = @(
    "avcodec-61.dll",
    "avformat-61.dll",
    "avutil-59.dll",
    "swscale-8.dll",
    "swresample-5.dll",
    "avdevice-61.dll",
    "avfilter-10.dll"
)
foreach ($dll in $FfmpegDlls) {
    $src = Join-Path $VcpkgBin $dll
    if (Test-Path $src) {
        Copy-Item $src "$StageDir\$dll"
    } else {
        # Try glob pattern for version-agnostic match
        $base = ($dll -split '-')[0]
        $found = Get-ChildItem "$VcpkgBin\$base*.dll" -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($found) {
            Copy-Item $found.FullName "$StageDir\$($found.Name)"
            Write-Host "  (matched $($found.Name) for $dll)"
        } else {
            Write-Warning "FFmpeg DLL not found: $dll"
        }
    }
}

# Also copy any transitive DLL dependencies (e.g. zlib, bzip2, etc.)
$TransitiveDlls = Get-ChildItem "$VcpkgBin\*.dll" -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -notmatch "^(av|sw)" }
foreach ($dll in $TransitiveDlls) {
    if (-not (Test-Path "$StageDir\$($dll.Name)")) {
        Copy-Item $dll.FullName "$StageDir\$($dll.Name)"
    }
}

# --- Copy NDI DLL (optional) ---
$NdiDll = "C:\Program Files\NDI\NDI 6 Runtime\v6\Processing.NDI.Lib.x64.dll"
if (Test-Path $NdiDll) {
    Write-Host "==> Copying NDI runtime DLL..."
    Copy-Item $NdiDll "$StageDir\Processing.NDI.Lib.x64.dll"
} else {
    Write-Host "==> NDI runtime not found, skipping (NDI features will be disabled)"
}

# --- Copy licenses ---
Write-Host "==> Copying licenses..."
Copy-Item "LICENSE" "$StageDir\LICENSE"
if (Test-Path "FFMPEG-LICENSE") {
    Copy-Item "FFMPEG-LICENSE" "$StageDir\FFMPEG-LICENSE"
} else {
    # Create a minimal FFmpeg license notice
    @"
FFmpeg is licensed under the GNU Lesser General Public License (LGPL) version 2.1 or later.
See https://ffmpeg.org/legal.html for details.

The FFmpeg shared libraries bundled with Varda are dynamically linked,
preserving LGPL compliance. Source code is available at https://ffmpeg.org.
"@ | Set-Content "$StageDir\FFMPEG-LICENSE"
}

# --- Create ZIP ---
$ZipPath = "$ProjectRoot\Varda-Windows-x64.zip"
if (Test-Path $ZipPath) { Remove-Item $ZipPath }
Write-Host "==> Creating $ZipPath..."
Compress-Archive -Path $StageDir -DestinationPath $ZipPath

Write-Host "==> Done! Artifact: $ZipPath"
Get-ChildItem $StageDir | Format-Table Name, Length -AutoSize
