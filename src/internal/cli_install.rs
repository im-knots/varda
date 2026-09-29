//! First-launch CLI install: when running from a .app bundle or `AppImage`,
//! puts a `varda` command on the user's PATH.
//!
//! - **macOS**: writes a wrapper script at `/usr/local/bin/varda` that sets
//!   `DYLD_FALLBACK_LIBRARY_PATH` and execs the binary inside the .app. The
//!   admin prompt comes from `osascript`.
//! - **Linux**: symlinks the `AppImage` to `~/.local/bin/varda`.

#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::path::Path;
#[cfg(target_os = "linux")]
use std::path::PathBuf;

/// Runs the first-launch CLI install. Silent on success, non-fatal on failure.
pub fn ensure_cli_installed() {
    if let Err(e) = try_install() {
        log::debug!("CLI install check skipped: {e}");
    }
}

fn try_install() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let exe = exe.canonicalize().unwrap_or_else(|_| exe.clone());

    #[cfg(target_os = "macos")]
    {
        install_macos(&exe)
    }
    #[cfg(target_os = "linux")]
    {
        install_linux(&exe)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = exe;
        Err("unsupported platform".into())
    }
}

// ---------------------------------------------------------------------------
// macOS: /Applications/Varda.app/Contents/MacOS/varda
//        → /usr/local/bin/varda (wrapper script, needs admin)
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
fn install_macos(exe: &Path) -> Result<(), String> {
    let macos_dir = exe.parent().ok_or("no parent")?;
    if macos_dir.file_name().and_then(|n| n.to_str()) != Some("MacOS") {
        return Err("not running from .app bundle".into());
    }
    let contents_dir = macos_dir.parent().ok_or("no Contents dir")?;
    let app_dir = contents_dir.parent().ok_or("no .app dir")?;
    if app_dir.extension().is_none_or(|ext| ext != "app") {
        return Err("not a .app bundle".into());
    }

    let wrapper = Path::new("/usr/local/bin/varda");
    if wrapper.exists() {
        if let Ok(contents) = std::fs::read_to_string(wrapper)
            && contents.contains(&exe.to_string_lossy().to_string())
        {
            return Ok(()); // already installed for this .app
        }
        // Not our wrapper; leave it alone.
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

#[cfg(target_os = "macos")]
fn install_macos_with_admin(wrapper_content: &str) -> Result<(), String> {
    // osascript shows the GUI admin prompt.
    let escaped = wrapper_content.replace('\\', "\\\\").replace('"', "\\\"");
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
        .map_err(|e| format!("osascript failed: {e}"))?;

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

#[cfg(target_os = "linux")]
fn install_linux(exe: &Path) -> Result<(), String> {
    let appimage_var = std::env::var("APPIMAGE").ok();
    let appimage_path = appimage_var
        .as_deref()
        .map_or_else(|| exe.to_path_buf(), PathBuf::from);

    // AppImage sets $APPIMAGE.
    if appimage_var.is_none() {
        return Err("not running from AppImage".into());
    }

    let home = std::env::var("HOME").map_err(|_| "no $HOME")?;
    let bin_dir = PathBuf::from(&home).join(".local/bin");
    let link_path = bin_dir.join("varda");

    if link_path.exists() {
        if let Ok(target) = std::fs::read_link(&link_path)
            && target == appimage_path
        {
            return Ok(()); // already installed
        }
        return Err("existing ~/.local/bin/varda not managed by this install".into());
    }

    log::info!("Installing CLI symlink to ~/.local/bin/varda...");
    std::fs::create_dir_all(&bin_dir).map_err(|e| format!("mkdir ~/.local/bin: {e}"))?;
    std::os::unix::fs::symlink(&appimage_path, &link_path).map_err(|e| format!("symlink: {e}"))?;

    log::info!(
        "CLI symlink installed: {} → {}",
        link_path.display(),
        appimage_path.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_cli_installed_does_not_panic() {
        // Under cargo test this is neither a .app nor an AppImage, so it skips.
        ensure_cli_installed();
    }

    #[test]
    fn try_install_skips_when_not_bundled() {
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
            "unexpected error: {msg}"
        );
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn linux_rejects_non_appimage() {
        // SAFETY: no other test touches APPIMAGE, so this cannot race another
        // test thread.
        unsafe {
            std::env::remove_var("APPIMAGE");
        }
        let exe = std::env::current_exe().unwrap();
        let result = install_linux(&exe);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not running from AppImage"));
    }
}
