//! First-launch CLI installation.
//!
//! Detects when varda is running from an installed location (.app bundle or
//! AppImage) and ensures a `varda` command is available in the user's PATH.
//!
//! - **macOS**: creates a wrapper script in `/usr/local/bin/varda` that sets
//!   `DYLD_FALLBACK_LIBRARY_PATH` and execs the binary inside the .app.
//!   Uses `osascript` for the admin prompt.
//! - **Linux**: symlinks the AppImage to `~/.local/bin/varda`.

use std::path::{Path, PathBuf};

/// Run the first-launch CLI install check.
/// This is intentionally silent on success and non-fatal on failure.
pub fn ensure_cli_installed() {
    if let Err(e) = try_install() {
        log::debug!("CLI install check skipped: {}", e);
    }
}

fn try_install() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {}", e))?;
    let exe = exe
        .canonicalize()
        .unwrap_or_else(|_| exe.clone());

    if cfg!(target_os = "macos") {
        install_macos(&exe)
    } else if cfg!(target_os = "linux") {
        install_linux(&exe)
    } else {
        Err("unsupported platform".into())
    }
}

// ---------------------------------------------------------------------------
// macOS: /Applications/Varda.app/Contents/MacOS/varda
//        → /usr/local/bin/varda (wrapper script, needs admin)
// ---------------------------------------------------------------------------

fn install_macos(exe: &Path) -> Result<(), String> {
    // Only act when running from a .app bundle
    let macos_dir = exe.parent().ok_or("no parent")?;
    if macos_dir.file_name().and_then(|n| n.to_str()) != Some("MacOS") {
        return Err("not running from .app bundle".into());
    }
    let contents_dir = macos_dir.parent().ok_or("no Contents dir")?;
    let app_dir = contents_dir.parent().ok_or("no .app dir")?;
    if !app_dir
        .extension()
        .map_or(false, |ext| ext == "app")
    {
        return Err("not a .app bundle".into());
    }

    let wrapper = Path::new("/usr/local/bin/varda");
    if wrapper.exists() {
        // Check if existing wrapper points to this .app
        if let Ok(contents) = std::fs::read_to_string(wrapper) {
            if contents.contains(&exe.to_string_lossy().to_string()) {
                return Ok(()); // already installed for this .app
            }
        }
        // Different install or not our wrapper — leave it alone
        return Err("existing /usr/local/bin/varda not managed by this install".into());
    }

    let frameworks = contents_dir.join("Frameworks");
    let wrapper_content = format!(
        "#!/bin/bash\n\
         # Varda CLI wrapper — auto-installed on first launch\n\
         export DYLD_FALLBACK_LIBRARY_PATH=\"{}:${{DYLD_FALLBACK_LIBRARY_PATH:-}}\"\n\
         exec \"{}\" \"$@\"\n",
        frameworks.display(),
        exe.display(),
    );

    log::info!("Installing CLI wrapper to /usr/local/bin/varda...");
    install_macos_with_admin(&wrapper_content)
}

fn install_macos_with_admin(wrapper_content: &str) -> Result<(), String> {
    // Use osascript to get admin privileges via GUI prompt
    let escaped = wrapper_content
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let script = format!(
        "do shell script \
         \"mkdir -p /usr/local/bin && \
         printf '{}' > /usr/local/bin/varda && \
         chmod +x /usr/local/bin/varda\" \
         with administrator privileges",
        escaped.replace('\'', "'\\''"),
    );

    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .map_err(|e| format!("osascript failed: {}", e))?;

    if output.status.success() {
        log::info!("CLI wrapper installed: /usr/local/bin/varda");
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("User canceled") || stderr.contains("-128") {
            log::info!("User declined CLI install — skipping");
            Ok(())
        } else {
            Err(format!("osascript error: {}", stderr.trim()))
        }
    }
}

// ---------------------------------------------------------------------------
// Linux: /path/to/Varda-x86_64.AppImage → ~/.local/bin/varda (symlink)
// ---------------------------------------------------------------------------

fn install_linux(exe: &Path) -> Result<(), String> {
    // Only act when running from an AppImage
    let appimage_var = std::env::var("APPIMAGE").ok();
    let appimage_path = appimage_var
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| exe.to_path_buf());

    // Heuristic: AppImage sets $APPIMAGE env var
    if appimage_var.is_none() {
        return Err("not running from AppImage".into());
    }

    let home = std::env::var("HOME").map_err(|_| "no $HOME")?;
    let bin_dir = PathBuf::from(&home).join(".local/bin");
    let link_path = bin_dir.join("varda");

    if link_path.exists() {
        // Check if symlink already points to this AppImage
        if let Ok(target) = std::fs::read_link(&link_path) {
            if target == appimage_path {
                return Ok(()); // already installed
            }
        }
        return Err("existing ~/.local/bin/varda not managed by this install".into());
    }

    log::info!("Installing CLI symlink to ~/.local/bin/varda...");
    std::fs::create_dir_all(&bin_dir)
        .map_err(|e| format!("mkdir ~/.local/bin: {}", e))?;
    std::os::unix::fs::symlink(&appimage_path, &link_path)
        .map_err(|e| format!("symlink: {}", e))?;

    log::info!("CLI symlink installed: {} → {}", link_path.display(), appimage_path.display());
    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_cli_installed_does_not_panic() {
        // Running from cargo test, not from .app or AppImage — should silently skip
        ensure_cli_installed();
    }

    #[test]
    fn try_install_skips_when_not_bundled() {
        // Not running from .app or AppImage, should return Err (skipped)
        let result = try_install();
        assert!(result.is_err());
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn macos_rejects_non_app_bundle() {
        let exe = std::env::current_exe().unwrap();
        let result = install_macos(&exe);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(
            msg.contains("not running from .app bundle") || msg.contains("not a .app bundle"),
            "unexpected error: {}",
            msg
        );
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn linux_rejects_non_appimage() {
        // Ensure APPIMAGE is not set
        std::env::remove_var("APPIMAGE");
        let exe = std::env::current_exe().unwrap();
        let result = install_linux(&exe);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not running from AppImage"));
    }
}