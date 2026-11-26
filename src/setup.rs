// One-time setup helpers: keyboard access, start at login and menu entries.
// They are used by the Setup tab, mainly when the program runs from an AppImage.

use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
};

// The AppImage file itself (the AppImage runtime sets APPIMAGE), if we run from one
pub fn appimage_path() -> Option<PathBuf> {
    std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .filter(|p| p.is_file())
}

pub fn is_appimage() -> bool {
    appimage_path().is_some()
}

// The file that starts a new copy of this program: the AppImage, or the installed program.
// (Inside an AppImage the running program disappears when the window closes, so a background
// copy has to be started from the AppImage file itself.)
pub fn launcher_path() -> Option<PathBuf> {
    appimage_path().or_else(|| std::env::current_exe().ok())
}

// Can the program read the keyboard and type? (needs the "input" group and /dev/uinput)
pub fn keyboard_access_ok() -> bool {
    let can_type = fs::OpenOptions::new()
        .write(true)
        .open("/dev/uinput")
        .is_ok();
    let can_read = match fs::read_dir("/dev/input") {
        Ok(entries) => entries.flatten().any(|e| {
            e.file_name().to_string_lossy().starts_with("event") && fs::File::open(e.path()).is_ok()
        }),
        Err(_) => false,
    };
    can_type && can_read
}

const SETUP_SCRIPT: &str = r#"set -e
USER_NAME="$1"
printf '%s\n' 'KERNEL=="uinput", GROUP="input", MODE="0660", OPTIONS+="static_node=uinput"' > /etc/udev/rules.d/99-expander-uinput.rules
printf '%s\n' 'uinput' > /etc/modules-load.d/expander-uinput.conf
modprobe uinput || true
getent group input >/dev/null || groupadd input
usermod -aG input "$USER_NAME"
udevadm control --reload
udevadm trigger
"#;

// Asks for the password (pkexec) and sets up the keyboard access. A logout is needed afterwards.
pub fn grant_keyboard_access() -> Result<(), String> {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .map_err(|_| "Could not find your user name.".to_string())?;
    let status = Command::new("pkexec")
        .arg("sh")
        .arg("-c")
        .arg(SETUP_SCRIPT)
        .arg("expander-setup")
        .arg(&user)
        .stdin(Stdio::null())
        .status();
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(_) => Err("The setup was cancelled or did not finish.".to_string()),
        Err(_) => Err(format!(
            "pkexec was not found. Run this in a terminal instead: sudo usermod -aG input {} (and create /etc/udev/rules.d/99-expander-uinput.rules, see the README), then log out and in.",
            user
        )),
    }
}

fn autostart_file() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("autostart")
        .join("text-expander.desktop")
}

pub fn autostart_enabled() -> bool {
    autostart_file().is_file()
}

// Starts the background service at login (an XDG autostart entry that works on any desktop)
pub fn set_autostart(on: bool) -> Result<(), String> {
    let path = autostart_file();
    if !on {
        let _ = fs::remove_file(&path);
        return Ok(());
    }
    let launcher = launcher_path().ok_or_else(|| "Could not find this program.".to_string())?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = format!(
        "[Desktop Entry]\nType=Application\nName=Text Expander (background)\nExec=\"{}\" --daemon\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
        launcher.display()
    );
    fs::write(&path, text).map_err(|e| e.to_string())
}

// Adds Text Expander (and entries for desktop shortcuts) to the application menu
pub fn install_menu_entries() -> Result<(), String> {
    let launcher = launcher_path().ok_or_else(|| "Could not find this program.".to_string())?;
    let data = dirs::data_dir().ok_or_else(|| "Could not find your data folder.".to_string())?;
    let apps = data.join("applications");
    fs::create_dir_all(&apps).map_err(|e| e.to_string())?;

    // the icon (inside the AppImage)
    if let Some(appdir) = std::env::var_os("APPDIR") {
        let source = PathBuf::from(appdir).join("usr/share/icons/hicolor/256x256/apps/expander.png");
        if source.is_file() {
            let icon_dir = data.join("icons").join("hicolor").join("256x256").join("apps");
            let _ = fs::create_dir_all(&icon_dir);
            let _ = fs::copy(&source, icon_dir.join("text-expander.png"));
        }
    }

    let entries: [(&str, &str, &str, &str); 7] = [
        ("text-expander.desktop", "Text Expander", "", "Text expander, OCR, voice typing and read aloud"),
        ("text-expander-voice.desktop", "Text Expander: voice typing (start/stop)", "--voice-toggle", "Start or stop voice typing"),
        ("text-expander-ocr.desktop", "Text Expander: read text from the screen (OCR)", "--ocr", "Select an area and copy its text"),
        ("text-expander-read-da.desktop", "Text Expander: read aloud in Danish", "--read-da", "Read the selected text aloud in Danish"),
        ("text-expander-read-en.desktop", "Text Expander: read aloud in English", "--read-en", "Read the selected text aloud in English"),
        ("text-expander-read-stop.desktop", "Text Expander: stop reading", "--read-stop", "Stop reading aloud"),
        ("text-expander-read-pause.desktop", "Text Expander: pause / resume reading", "--read-pause", "Pause or resume reading aloud"),
    ];
    for (file, name, flag, comment) in entries {
        let exec = if flag.is_empty() {
            format!("\"{}\"", launcher.display())
        } else {
            format!("\"{}\" {}", launcher.display(), flag)
        };
        let text = format!(
            "[Desktop Entry]\nType=Application\nName={}\nComment={}\nExec={}\nIcon=text-expander\nTerminal=false\nCategories=Utility;\n",
            name, comment, exec
        );
        fs::write(apps.join(file), text).map_err(|e| e.to_string())?;
    }
    Ok(())
}
