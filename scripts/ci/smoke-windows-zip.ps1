# Verify a Varda Windows portable ZIP extracts and runs.
# Usage: .\scripts\ci\smoke-windows-zip.ps1 -Zip <path-to-Varda-Windows-x64.zip>
#
# Runs on a CLEAN runner, not the one that built it. That is the whole point: the build
# runner has vcpkg, the NDI Runtime and LLVM installed, so a DLL we forgot to bundle
# resolves there and nowhere else. See spec/release-strategy.md section 12.
param(
    [Parameter(Mandatory = $true)][string]$Zip
)
$ErrorActionPreference = 'Stop'

$work = Join-Path $env:RUNNER_TEMP "varda-smoke-$(Get-Random)"
New-Item -ItemType Directory -Path $work -Force | Out-Null

Write-Host "==> Host"
Write-Host "    $((Get-CimInstance Win32_OperatingSystem).Caption), $env:PROCESSOR_ARCHITECTURE"
Write-Host "==> Extracting $(Split-Path $Zip -Leaf)"
Expand-Archive -Path $Zip -DestinationPath $work -Force

$exe = Get-ChildItem -Path $work -Filter 'varda.exe' -Recurse | Select-Object -First 1
if (-not $exe) { Write-Error "::error::no varda.exe inside the ZIP"; exit 1 }
$root = $exe.Directory.FullName
Write-Host "    root: $root"

$failed = $false

# --- Shaders shipped where the binary looks for them ---
# On Windows the binary resolves shaders/ next to varda.exe (BUNDLED_SHADERS_RELATIVE in
# src/internal/registry/mod.rs). Drift means an app that starts with an empty library.
Write-Host "==> Checking the shader library"
$shaderDir = Join-Path $root 'shaders'
$count = 0
if (Test-Path $shaderDir) {
    $count = (Get-ChildItem -Path $shaderDir -Filter '*.fs' -Recurse -ErrorAction SilentlyContinue).Count
}
if ($count -lt 100) {
    Write-Host "FAIL: found $count bundled shaders in shaders\, expected the full library"
    $failed = $true
} else {
    Write-Host "  ok: $count shaders bundled"
}

# --- DLLs present ---
Write-Host "==> Bundled DLLs"
$dlls = Get-ChildItem -Path $root -Filter '*.dll' -ErrorAction SilentlyContinue
Write-Host "    $($dlls.Count) DLL(s) alongside varda.exe"
if ($dlls.Count -eq 0) {
    Write-Host "FAIL: no DLLs bundled; FFmpeg is dynamically linked and must ship with the ZIP"
    $failed = $true
}

# --- It actually launches ---
# Windows has no ldd. Running the binary is the dependency check: a missing DLL aborts
# the process at load with STATUS_DLL_NOT_FOUND (0xC0000135) before main() runs.
# --version is handled by clap and returns before any GPU, window or audio init.
Write-Host "==> Launching"
$out = & $exe.FullName --version 2>&1
$code = $LASTEXITCODE
if ($code -eq 0) {
    Write-Host "  ok: $out"
} else {
    Write-Host "FAIL: varda.exe --version exited $code"
    $out | ForEach-Object { Write-Host "    $_" }
    if ($code -eq -1073741515) {
        Write-Host "    (0xC0000135 STATUS_DLL_NOT_FOUND: a required DLL is not in the ZIP)"
    }
    $failed = $true
}

Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue

if ($failed) {
    Write-Host "==> SMOKE TEST FAILED"
    exit 1
}
Write-Host "==> SMOKE TEST PASSED"
