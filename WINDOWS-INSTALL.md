# Windows Installation Guide for Varda

This guide covers all prerequisites and dependencies required to build and run Varda on Windows.

## Table of Contents
- [Quick Install Summary](#quick-install-summary)
- [Detailed Prerequisites](#detailed-prerequisites)
- [Installation Steps](#installation-steps)
- [Environment Variables](#environment-variables)
- [Optional Features](#optional-features)
- [Troubleshooting](#troubleshooting)

---

## Quick Install Summary

**Required:**
1. [Rust (stable)](#1-install-rust)
2. [Visual Studio Build Tools](#2-visual-studio-build-tools) (or full Visual Studio)
3. [LLVM/Clang](#3-llvmclang) (for shader compilation)
4. [FFmpeg](#4-ffmpeg) (for video/audio)
5. [vcpkg](#5-vcpkg-package-manager) (package manager for C++ libraries)

**Optional:**
- [NDI SDK](#optional-ndi-support) (for NDI send/receive)

---

## Detailed Prerequisites

### System Requirements
- **OS:** Windows 10 or Windows 11 (64-bit)
- **GPU:** DirectX 12 compatible GPU (for wgpu rendering)
- **RAM:** 8GB minimum, 16GB recommended
- **Disk Space:** ~10GB for all dependencies + build artifacts

### Required Dependencies

The following crates require native system libraries:

| Crate | System Library | Used For |
|-------|---------------|----------|
| `shaderc` | LLVM/Clang | ISF/GLSL shader compilation |
| `ffmpeg-next` | FFmpeg (libavcodec, libavformat, libavutil, libswscale, libswresample, libavdevice) | Video/audio decoding, HAP codec, SRT streaming |
| `nokhwa` | Windows Media Foundation | Webcam capture (built into Windows 10/11) |
| `midir` | Windows Multimedia API | MIDI device support (built into Windows) |
| `cpal` | WASAPI | Audio analysis (built into Windows) |

---

## Installation Steps

### 1. Install Rust

Download and install Rust using `rustup`:

```powershell
# Download and run rustup-init.exe from:
# https://rustup.rs/

# Or use winget:
winget install Rustlang.Rustup
```

Verify installation:
```powershell
rustc --version
cargo --version
```

---

### 2. Visual Studio Build Tools

Rust on Windows requires the MSVC (Microsoft Visual C++) toolchain. **IMPORTANT:** You also need ATL (Active Template Library) for FFmpeg dependencies.

**Option A: Visual Studio Build Tools (Lighter)**
1. Download: https://visualstudio.microsoft.com/downloads/#build-tools-for-visual-studio-2022
2. During installation:
   - ✅ Check **"Desktop development with C++"** workload
   - Go to **"Individual components"** tab
   - Search for "ATL" and check:
     - ✅ **C++ ATL for latest v143 build tools (x86 & x64)**
     - ✅ **C++ MFC for latest v143 build tools (x86 & x64)** (recommended)

**Option B: Visual Studio Community (Full IDE)**
1. Download: https://visualstudio.microsoft.com/vs/community/
2. During installation:
   - ✅ Select **"Desktop development with C++"** workload
   - Go to **"Individual components"** tab
   - Search for "ATL" and check:
     - ✅ **C++ ATL for latest v143 build tools (x86 & x64)**
     - ✅ **C++ MFC for latest v143 build tools (x86 & x64)** (recommended)

**If you already have Visual Studio installed but missing ATL:**

```powershell
# Open Visual Studio Installer
& "C:\Program Files (x86)\Microsoft Visual Studio\Installer\vs_installer.exe"
```

Then:
1. Click **"Modify"** next to your Visual Studio installation
2. Go to **"Individual components"** tab
3. Search for "ATL"
4. Check:
   - ✅ **C++ ATL for latest v143 build tools (x86 & x64)**
   - ✅ **C++ MFC for latest v143 build tools (x86 & x64)**
5. Click **"Modify"** to install

---

### 3. LLVM/Clang

Required by `shaderc` for shader compilation. You have two options:

#### Option A: Official LLVM Installer (Recommended - Faster)

1. **Download LLVM:**
   - Visit: https://github.com/llvm/llvm-project/releases/latest
   - Download: `LLVM-<version>-win64.exe` (e.g., `LLVM-18.1.0-win64.exe`)

2. **Install:**
   - Run the installer
   - ✅ Check "Add LLVM to the system PATH for all users" (or current user)
   - Install to default location: `C:\Program Files\LLVM`

3. **Set environment variable:**
   ```powershell
   # For current session:
   $env:LIBCLANG_PATH = "C:\Program Files\LLVM\bin"
   
   # To make permanent (run PowerShell as Administrator):
   [System.Environment]::SetEnvironmentVariable('LIBCLANG_PATH', 'C:\Program Files\LLVM\bin', [System.EnvironmentVariableTarget]::Machine)
   ```

#### Option B: Install via vcpkg

```powershell
cd vcpkg
.\vcpkg install llvm:x64-windows
```

Then set:
```powershell
$env:LIBCLANG_PATH = "C:\path\to\vcpkg\installed\x64-windows\bin"
```

---

### 4. FFmpeg

Required by `ffmpeg-next` for video decoding, encoding, and streaming.

#### Using vcpkg (Recommended for Windows)

```powershell
# Navigate to your vcpkg directory
cd vcpkg

# Bootstrap vcpkg if not already done
.\bootstrap-vcpkg.bat

# Install FFmpeg (this will take 10-30 minutes on first install)
.\vcpkg install ffmpeg:x64-windows

# Install pkg-config (required for finding libraries)
.\vcpkg install pkgconf:x64-windows

# Integrate vcpkg with your user account
.\vcpkg integrate install
```

**Set environment variable:**
```powershell
# For current session:
$env:VCPKG_ROOT = "C:\path\to\vcpkg"

# To make permanent:
[System.Environment]::SetEnvironmentVariable('VCPKG_ROOT', 'C:\path\to\vcpkg', [System.EnvironmentVariableTarget]::User)
```

---

### 5. vcpkg Package Manager

If you don't already have vcpkg set up:

```powershell
# Clone vcpkg
cd C:\path\to  # or wherever you keep projects
git clone https://github.com/microsoft/vcpkg.git
cd vcpkg

# Bootstrap vcpkg
.\bootstrap-vcpkg.bat

# Integrate with Visual Studio (optional but recommended)
.\vcpkg integrate install
```

---

## Environment Variables

After installing all dependencies, ensure these environment variables are set:

### Required:

```powershell
# LLVM for shader compilation
$env:LIBCLANG_PATH = "C:\Program Files\LLVM\bin"

# vcpkg for FFmpeg and other libraries
$env:VCPKG_ROOT = "C:\path\to\vcpkg"
```

### Making Environment Variables Permanent:

**Via PowerShell (Administrator):**
```powershell
[System.Environment]::SetEnvironmentVariable('LIBCLANG_PATH', 'C:\Program Files\LLVM\bin', [System.EnvironmentVariableTarget]::User)
[System.Environment]::SetEnvironmentVariable('VCPKG_ROOT', 'C:\path\to\vcpkg', [System.EnvironmentVariableTarget]::User)
```

**Via GUI:**
1. Press `Win + R`, type `sysdm.cpl`, press Enter
2. Go to "Advanced" tab → "Environment Variables"
3. Under "User variables", click "New"
4. Add:
   - Variable: `LIBCLANG_PATH`, Value: `C:\Program Files\LLVM\bin`
   - Variable: `VCPKG_ROOT`, Value: `C:\path\to\vcpkg`

---

## Building Varda

Once all dependencies are installed:

```powershell
# Navigate to varda directory
cd varda

# Clean any previous build artifacts
cargo clean

# Build in release mode (recommended)
cargo build --release

# Or run directly
cargo run --release
```

---

## Optional Features

### Optional: NDI Support

NDI (Network Device Interface) enables sending and receiving video over the network. It's **optional** – Varda will work without it, but NDI sources will be unavailable.

**Installation:**

1. **Download NDI SDK:**
   - Visit: https://ndi.video/tools/
   - Download "NDI SDK" for Windows
   - Account registration required (free)

2. **Install:**
   - Run the installer
   - Default location: `C:\Program Files\NDI\NDI 6 SDK`

3. **Configure:**
   The NDI SDK is dynamically loaded at runtime. Ensure the NDI runtime DLLs are in your system PATH or in the same directory as the Varda executable.

   ```powershell
   # Add NDI to PATH (adjust version as needed)
   $env:PATH = "C:\Program Files\NDI\NDI 6 SDK\Bin\x64;$env:PATH"
   ```

4. **Verify:**
   After building Varda, NDI features will be automatically detected. Check the console output on startup:
   - ✅ "NDI SDK initialized successfully" = NDI available
   - ℹ️ "NDI SDK not found — NDI features disabled" = NDI unavailable (but app works normally)

**Note:** Without the NDI SDK, NDI send/receive options in the UI will be hidden/disabled. All other features work normally.

---

## Troubleshooting

### Issue: `unable to find libclang`

**Error:**
```
Unable to find libclang: "couldn't find any valid shared libraries matching: ['clang.dll', 'libclang.dll']"
```

**Solution:**
- Ensure LLVM is installed (see [Step 3](#3-llvmclang))
- Set `LIBCLANG_PATH` environment variable
- Restart your terminal/PowerShell after setting environment variables

**Verify:**
```powershell
# Check if libclang.dll exists
Test-Path "C:\Program Files\LLVM\bin\libclang.dll"

# Should output: True
```

---

### Issue: `pkg-config exited with status code 1`

**Error:**
```
The system library `libavutil` required by crate `ffmpeg-sys-next` was not found.
```

**Solution:**
- Install FFmpeg via vcpkg (see [Step 4](#4-ffmpeg))
- Install pkgconf: `.\vcpkg install pkgconf:x64-windows`
- Set `VCPKG_ROOT` environment variable
- Run `.\vcpkg integrate install`

---

### Issue: `Unable to locate 'atlbase.h'` or `Building atl:x64-windows failed`

**Error:**
```
CMake Error at ports/atl/portfile.cmake:7 (message):
Unable to locate 'atlbase.h'.  Ensure you have installed the Active
Template Library (ATL) component of Visual Studio.
```

**Solution:**
The Active Template Library (ATL) is missing from your Visual Studio installation.

**Fix via Visual Studio Installer:**

```powershell
# Open Visual Studio Installer
& "C:\Program Files (x86)\Microsoft Visual Studio\Installer\vs_installer.exe"
```

Then:
1. Click **"Modify"** next to Visual Studio 2022 Community (or Build Tools)
2. Go to **"Individual components"** tab
3. Search for "ATL"
4. Check these boxes:
   - ✅ **C++ ATL for latest v143 build tools (x86 & x64)**
   - ✅ **C++ MFC for latest v143 build tools (x86 & x64)**
5. Click **"Modify"** and wait for installation
6. Retry vcpkg:
   ```powershell
   cd vcpkg
   .\.vcpkg install ffmpeg:x64-windows pkgconf:x64-windows
   ```

**Alternative: Use static builds (no ATL required):**

If you don't want to install ATL:

```powershell
cd vcpkg
.\.vcpkg install ffmpeg[core]:x64-windows-static pkgconf:x64-windows
```

Then when building Varda:
```powershell
$env:VCPKGRS_DYNAMIC = "0"
cargo build --release
```

---

### Issue: `link.exe not found` or `cl.exe not found`

**Error:**
```
error: linker `link.exe` not found
```

**Solution:**
- Install Visual Studio Build Tools (see [Step 2](#2-visual-studio-build-tools))
- Ensure "Desktop development with C++" workload is installed
- Ensure ATL components are installed (see above)
- Restart your terminal after installation

---

### Issue: Build is very slow on first run

**This is normal.** The first build:
- Downloads and compiles all Rust dependencies (~200+ crates)
- Can take 10-30 minutes depending on your CPU
- Subsequent builds are much faster (incremental compilation)

**Tips:**
- Use `cargo build --release` for production builds (slower build, faster runtime)
- Use `cargo build` for development (faster builds, slower runtime)
- Use `cargo check` for fast syntax checking without building

---

### Issue: NDI sources not appearing

**Symptoms:**
- Varda runs but NDI sources don't show up
- Console shows "NDI SDK not found"

**Solution:**
- NDI is optional. Verify if you need it.
- If needed, install NDI SDK (see [Optional: NDI Support](#optional-ndi-support))
- Ensure NDI DLLs are in PATH or app directory

---

### Issue: `error: failed to run custom build command for wgpu`

**Error:**
```
error: failed to run custom build command for `wgpu-core`
```

**Solution:**
- Update your GPU drivers
- Ensure you have DirectX 12 support
- On older systems, wgpu may require Vulkan runtime: https://vulkan.lunarg.com/

---

### Issue: Webcam (nokhwa) not working

**Symptoms:**
- No webcams detected
- Camera permissions errors

**Solution:**
- Windows 10/11: Check Camera privacy settings
  - Settings → Privacy → Camera
  - Enable "Let apps access your camera"
- Ensure camera is not in use by another application
- Try unplugging and replugging USB cameras

---

### Issue: MIDI controllers not detected

**Symptoms:**
- MIDI devices don't appear in Varda

**Solution:**
- Ensure MIDI device drivers are installed (manufacturer website)
- Reconnect MIDI device
- Check Windows Device Manager for device status
- Some MIDI devices require specific drivers beyond the default Windows MIDI driver

---

### Issue: `error: could not compile` with firewall/antivirus warnings

**Solution:**
- Some antivirus software blocks Rust compiler operations
- Add exception for:
  - `cargo.exe`
  - `rustc.exe`
  - Your project directory
- Temporarily disable antivirus during first build (re-enable after)

---

## Complete Installation Script

Save this as `install-windows-deps.ps1` and run in PowerShell (as Administrator):

```powershell
# Varda Windows Dependencies Installer
# Run this in PowerShell as Administrator

Write-Host "=== Installing Varda Dependencies ===" -ForegroundColor Green

# 1. Check for Rust
Write-Host "`n1. Checking for Rust..." -ForegroundColor Cyan
if (!(Get-Command rustc -ErrorAction SilentlyContinue)) {
    Write-Host "Rust not found. Please install from: https://rustup.rs/" -ForegroundColor Yellow
    Write-Host "Or run: winget install Rustlang.Rustup" -ForegroundColor Yellow
    exit 1
} else {
    Write-Host "✓ Rust is installed: $(rustc --version)" -ForegroundColor Green
}

# 2. Check for LLVM
Write-Host "`n2. Checking for LLVM..." -ForegroundColor Cyan
$llvmPath = "C:\Program Files\LLVM\bin\libclang.dll"
if (Test-Path $llvmPath) {
    Write-Host "✓ LLVM is installed" -ForegroundColor Green
    [System.Environment]::SetEnvironmentVariable('LIBCLANG_PATH', 'C:\Program Files\LLVM\bin', [System.EnvironmentVariableTarget]::Machine)
    Write-Host "✓ LIBCLANG_PATH environment variable set" -ForegroundColor Green
} else {
    Write-Host "⚠ LLVM not found at $llvmPath" -ForegroundColor Yellow
    Write-Host "Download from: https://github.com/llvm/llvm-project/releases/latest" -ForegroundColor Yellow
    Write-Host "Install, then re-run this script" -ForegroundColor Yellow
    exit 1
}

# 3. Setup vcpkg
Write-Host "`n3. Setting up vcpkg..." -ForegroundColor Cyan
$vcpkgRoot = "C:\vcpkg"
if (!(Test-Path "$vcpkgRoot\vcpkg.exe")) {
    Write-Host "Cloning vcpkg..." -ForegroundColor Yellow
    git clone https://github.com/microsoft/vcpkg.git $vcpkgRoot
    Set-Location $vcpkgRoot
    .\bootstrap-vcpkg.bat
} else {
    Write-Host "✓ vcpkg already installed" -ForegroundColor Green
    Set-Location $vcpkgRoot
}

# Set VCPKG_ROOT environment variable
[System.Environment]::SetEnvironmentVariable('VCPKG_ROOT', $vcpkgRoot, [System.EnvironmentVariableTarget]::Machine)
Write-Host "✓ VCPKG_ROOT environment variable set" -ForegroundColor Green

# 4. Install FFmpeg and pkgconf
Write-Host "`n4. Installing FFmpeg and pkgconf via vcpkg..." -ForegroundColor Cyan
Write-Host "This may take 10-30 minutes on first run..." -ForegroundColor Yellow
.\vcpkg install ffmpeg:x64-windows pkgconf:x64-windows

# Integrate vcpkg
.\vcpkg integrate install

Write-Host "`n=== Installation Complete ===" -ForegroundColor Green
Write-Host "Environment variables set:" -ForegroundColor Cyan
Write-Host "  LIBCLANG_PATH = C:\Program Files\LLVM\bin" -ForegroundColor White
Write-Host "  VCPKG_ROOT = $vcpkgRoot" -ForegroundColor White
Write-Host "`nPlease restart your terminal/PowerShell for changes to take effect." -ForegroundColor Yellow
Write-Host "`nTo build Varda, run:" -ForegroundColor Cyan
Write-Host "  cd varda" -ForegroundColor White
Write-Host "  cargo build --release" -ForegroundColor White
```

---

## Verified Build Command Sequence

After all dependencies are installed and environment variables are set:

```powershell
# 1. Open a NEW PowerShell window (to load environment variables)

# 2. Navigate to Varda
cd C:\path\to\varda

# 3. Clean previous builds
cargo clean

# 4. Build release version
cargo build --release

# 5. Run Varda
.\target\release\varda.exe

# Or build and run in one command:
cargo run --release
```

---

## Next Steps

After successful installation:

1. **Read the main README.md** for usage instructions
2. **Create a workspace directory** for your projects
3. **Add ISF shaders** to `<workspace>/shaders/` directory
4. **Configure MIDI controllers** (auto-detected on first connection)
5. **Explore the UI** - check out the library, mixer, and effects panels

---

## Additional Resources

- **Varda Documentation:** See main `README.md`
- **Rust Installation:** https://rustup.rs/
- **vcpkg Documentation:** https://vcpkg.io/
- **FFmpeg Documentation:** https://ffmpeg.org/documentation.html
- **NDI SDK:** https://ndi.video/
- **ISF Shaders:** https://www.interactiveshaderformat.com/

---

## Platform Notes

While Varda is primarily designed for **Linux and macOS**, this Windows build is experimental. Known limitations:

- **No Syphon support** (macOS-only technology)
- **NDI support** requires Windows NDI SDK installation
- **SRT streaming** uses FFmpeg subprocess (requires ffmpeg.exe in PATH or beside app)
- **Performance** may vary depending on GPU driver quality for wgpu/DirectX 12

For the best experience, consider using **Linux** or **macOS** as development platforms.

---

**Last Updated:** 2024  
**Varda Version:** 0.1.0  
**Tested On:** Windows 11 22H2, Windows 10 22H2
