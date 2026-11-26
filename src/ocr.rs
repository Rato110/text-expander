use super::OcrConfig;
use std::{
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
};

static RUNNING: AtomicBool = AtomicBool::new(false);

pub fn have(program: &str) -> bool {
    match std::env::var_os("PATH") {
        Some(paths) => std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()),
        None => false,
    }
}

// A command that can reach the screen and clipboard even when it is started
// from the background service (which may not have the desktop's variables).
pub fn gui_cmd(program: &str) -> Command {
    let mut cmd = Command::new(program);
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        let dir = PathBuf::from(dir);
        if std::env::var_os("WAYLAND_DISPLAY").is_none() {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for e in entries.flatten() {
                    let name = e.file_name().to_string_lossy().to_string();
                    if name.starts_with("wayland-") && !name.ends_with(".lock") {
                        cmd.env("WAYLAND_DISPLAY", &name);
                        break;
                    }
                }
            }
        }
        if std::env::var_os("XAUTHORITY").is_none() {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for e in entries.flatten() {
                    let name = e.file_name().to_string_lossy().to_string();
                    if name.starts_with("xauth_") {
                        cmd.env("XAUTHORITY", e.path());
                        break;
                    }
                }
            }
        }
    }
    if std::env::var_os("DISPLAY").is_none() {
        let mut display = String::from(":0");
        if let Ok(entries) = std::fs::read_dir("/tmp/.X11-unix") {
            let mut names: Vec<String> = entries
                .flatten()
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|n| n.starts_with('X'))
                .collect();
            names.sort();
            if let Some(first) = names.first() {
                display = format!(":{}", &first[1..]);
            }
        }
        cmd.env("DISPLAY", display);
    }
    cmd
}

pub fn notify(title: &str, body: &str) {
    let _ = gui_cmd("notify-send")
        .args(["-a", "Text Expander", "-t", "5000", title, body])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

pub fn installed_languages() -> Vec<String> {
    match Command::new("tesseract").arg("--list-langs").output() {
        Ok(o) => {
            let mut text = String::from_utf8_lossy(&o.stdout).to_string();
            text.push('\n');
            text.push_str(&String::from_utf8_lossy(&o.stderr));
            text.lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty() && !l.starts_with("List of"))
                .collect()
        }
        Err(_) => Vec::new(),
    }
}

pub fn requirements() -> Vec<(String, bool)> {
    vec![
        ("Tesseract (reads the text)".to_string(), have("tesseract")),
        (
            "Screenshot tool (spectacle, gnome-screenshot, grim + slurp, or maim)".to_string(),
            have("spectacle")
                || have("gnome-screenshot")
                || (have("grim") && have("slurp"))
                || have("maim"),
        ),
        (
            "Clipboard tool (KDE Klipper via gdbus, wl-copy, xclip or xsel)".to_string(),
            have("gdbus") || have("wl-copy") || have("xclip") || have("xsel"),
        ),
    ]
}

// Let the user pick an area; the picture is saved to `path`.
// Ok(true): picture taken. Ok(false): the user cancelled.
fn capture(path: &str) -> Result<bool, String> {
    let _ = std::fs::remove_file(path);
    if have("spectacle") {
        let _ = gui_cmd("spectacle")
            .args(["--background", "--nonotify", "--region", "--output", path])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    } else if have("gnome-screenshot") {
        let _ = gui_cmd("gnome-screenshot")
            .args(["-a", "-f", path])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    } else if have("grim") && have("slurp") {
        let area = gui_cmd("slurp").output().map_err(|e| format!("slurp failed: {}", e))?;
        if !area.status.success() {
            return Ok(false);
        }
        let geometry = String::from_utf8_lossy(&area.stdout).trim().to_string();
        let _ = gui_cmd("grim")
            .arg("-g")
            .arg(&geometry)
            .arg(path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    } else if have("maim") {
        let _ = gui_cmd("maim")
            .args(["-s", path])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    } else {
        return Err("No screenshot tool found. Install spectacle (KDE), gnome-screenshot, grim + slurp, or maim.".to_string());
    }
    Ok(std::fs::metadata(path).map(|m| m.len() > 0).unwrap_or(false))
}

// Bigger, grayscale pictures are read more accurately (needs ImageMagick; optional)
fn enlarge(src: &str, dst: &str) -> bool {
    let tool = if have("magick") {
        "magick"
    } else if have("convert") {
        "convert"
    } else {
        return false;
    };
    Command::new(tool)
        .args([src, "-colorspace", "Gray", "-resize", "200%", dst])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn recognize(img: &str, languages: &str) -> Result<String, String> {
    if !have("tesseract") {
        return Err("Tesseract is not installed. Fedora: sudo dnf install tesseract tesseract-langpack-dan".to_string());
    }
    let installed = installed_languages();
    let wanted: Vec<&str> = languages
        .split('+')
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && installed.iter().any(|i| i == l))
        .collect();
    let lang_arg = if wanted.is_empty() {
        match installed.iter().find(|l| l.as_str() != "osd") {
            Some(l) => l.clone(),
            None => return Err("Tesseract has no language data installed.".to_string()),
        }
    } else {
        wanted.join("+")
    };
    let out = Command::new("tesseract")
        .arg(img)
        .arg("stdout")
        .arg("-l")
        .arg(&lang_arg)
        .output()
        .map_err(|e| format!("Could not run tesseract: {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "Tesseract failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .replace('\u{c}', "")
        .trim()
        .to_string())
}

fn pipe_to(program: &str, args: &[&str], text: &str) -> bool {
    let mut child = match gui_cmd(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    if let Some(mut stdin) = child.stdin.take() {
        if stdin.write_all(text.as_bytes()).is_err() {
            return false;
        }
    }
    child.wait().map(|s| s.success()).unwrap_or(false)
}

// Text as a GVariant string literal (for gdbus)
fn gvariant_quote(s: &str) -> String {
    let mut out = String::from("'");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            _ => out.push(c),
        }
    }
    out.push('\'');
    out
}

fn copy_to_clipboard(text: &str) -> bool {
    // KDE: ask Klipper (works on Wayland and X11)
    if have("gdbus") {
        let ok = gui_cmd("gdbus")
            .args([
                "call",
                "--session",
                "--dest",
                "org.kde.klipper",
                "--object-path",
                "/klipper",
                "--method",
                "org.kde.klipper.klipper.setClipboardContents",
            ])
            .arg(gvariant_quote(text))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            return true;
        }
    }
    if have("wl-copy") && pipe_to("wl-copy", &[], text) {
        return true;
    }
    if have("xclip") && pipe_to("xclip", &["-selection", "clipboard"], text) {
        return true;
    }
    if have("xsel") && pipe_to("xsel", &["--clipboard", "--input"], text) {
        return true;
    }
    false
}

// Select area -> read text -> copy to clipboard.
// Err("") means the user cancelled the selection.
pub fn run_once(cfg: OcrConfig) -> Result<String, String> {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err("OCR is already running.".to_string());
    }
    let result = run_inner(&cfg);
    RUNNING.store(false, Ordering::SeqCst);
    result
}

fn run_inner(cfg: &OcrConfig) -> Result<String, String> {
    let dir = std::env::temp_dir();
    let pid = std::process::id();
    let shot = dir
        .join(format!("expander-ocr-{}.png", pid))
        .to_string_lossy()
        .to_string();
    let big = dir
        .join(format!("expander-ocr-{}-big.png", pid))
        .to_string_lossy()
        .to_string();

    if !capture(&shot)? {
        return Err(String::new());
    }
    let image = if enlarge(&shot, &big) { big.clone() } else { shot.clone() };
    let text = recognize(&image, &cfg.languages);
    let _ = std::fs::remove_file(&shot);
    let _ = std::fs::remove_file(&big);
    let text = text?;
    if text.is_empty() {
        return Err("No text found in the selected area.".to_string());
    }
    if !copy_to_clipboard(&text) {
        return Err("Could not copy to the clipboard (needs KDE Klipper, wl-clipboard or xclip).".to_string());
    }
    Ok(text)
}

// Used by the keyboard shortcut: result shown as a desktop notification
pub fn run_with_notification(cfg: OcrConfig) {
    match run_once(cfg) {
        Ok(text) => {
            let preview: String = text.chars().take(80).collect();
            notify(
                "Text copied",
                &format!("{} characters copied: {}", text.chars().count(), preview),
            );
        }
        Err(e) if e.is_empty() => {}
        Err(e) => notify("OCR failed", &e),
    }
}
