#![cfg_attr(windows, windows_subsystem = "windows")]

use eframe::egui;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

#[derive(Serialize, Deserialize, Clone)]
struct Entry {
    trigger: String,
    text: String,
    // true: expands when you type the trigger and press Space
    // false: expands instantly as soon as the trigger is typed
    #[serde(default = "default_true")]
    space: bool,
}

fn default_true() -> bool {
    true
}

type Shared = Arc<Mutex<Vec<Entry>>>;
// "auto" or a layout code: "dk", "us", "uk", "fi"
type LayoutSetting = Arc<Mutex<String>>;

// OCR (copy the text of a selected screen area to the clipboard) settings
#[derive(Serialize, Deserialize, Clone)]
struct OcrConfig {
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default = "default_ocr_shortcut")]
    shortcut: String,
    #[serde(default = "default_ocr_languages")]
    languages: String,
}

fn default_ocr_shortcut() -> String {
    "Ctrl+Alt+O".to_string()
}

fn default_ocr_languages() -> String {
    "eng+dan".to_string()
}

impl Default for OcrConfig {
    fn default() -> Self {
        OcrConfig {
            enabled: true,
            shortcut: default_ocr_shortcut(),
            languages: default_ocr_languages(),
        }
    }
}

type OcrShared = Arc<Mutex<OcrConfig>>;

// Voice typing (offline speech-to-text) settings
#[derive(Serialize, Deserialize, Clone)]
struct VoiceConfig {
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default = "default_voice_shortcut")]
    shortcut: String,
    // "toggle": press to start, press again to stop. "hold": record while the key is held
    #[serde(default = "default_voice_mode")]
    mode: String,
    // "da", "en" or "auto" (used by the Whisper models; Parakeet detects the language itself)
    #[serde(default = "default_voice_language")]
    language: String,
    #[serde(default = "default_voice_model")]
    model: String,
    // show the text on screen while speaking
    #[serde(default)]
    live: bool,
    // "box" (window on screen) or "notification"
    #[serde(default = "default_live_style")]
    live_style: String,
    // short beeps when listening starts and stops
    #[serde(default = "default_true")]
    sounds: bool,
    // type each phrase as soon as the speaker pauses (toggle mode only)
    #[serde(default = "default_true")]
    type_live: bool,
    // how long a pause (in milliseconds) ends a phrase
    #[serde(default = "default_pause_ms")]
    pause_ms: u32,
    // show the text in a box where it can be corrected before it is typed
    #[serde(default)]
    review: bool,
}

fn default_voice_shortcut() -> String {
    "Ctrl+Alt+V".to_string()
}

fn default_voice_mode() -> String {
    "toggle".to_string()
}

fn default_voice_language() -> String {
    "da".to_string()
}

fn default_voice_model() -> String {
    "parakeet-onnx".to_string()
}

fn default_live_style() -> String {
    "box".to_string()
}

fn default_pause_ms() -> u32 {
    450
}

impl Default for VoiceConfig {
    fn default() -> Self {
        VoiceConfig {
            enabled: true,
            shortcut: default_voice_shortcut(),
            mode: default_voice_mode(),
            language: default_voice_language(),
            model: default_voice_model(),
            live: false,
            live_style: default_live_style(),
            sounds: true,
            type_live: true,
            pause_ms: default_pause_ms(),
            review: false,
        }
    }
}

type VoiceShared = Arc<Mutex<VoiceConfig>>;

// Read aloud (text to speech) settings
#[derive(Serialize, Deserialize, Clone)]
struct SpeakConfig {
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default = "default_speak_da")]
    read_da: String,
    #[serde(default = "default_speak_en")]
    read_en: String,
    #[serde(default = "default_speak_stop")]
    stop: String,
    #[serde(default = "default_speak_pause")]
    pause: String,
    // 1.0 = normal speed
    #[serde(default = "default_speak_speed")]
    speed: f32,
    #[serde(default = "default_voice_da")]
    voice_da: String,
    #[serde(default = "default_voice_en")]
    voice_en: String,
}

fn default_speak_da() -> String {
    "Ctrl+Alt+D".to_string()
}

fn default_speak_en() -> String {
    "Ctrl+Alt+E".to_string()
}

fn default_speak_stop() -> String {
    "Ctrl+Alt+S".to_string()
}

fn default_speak_pause() -> String {
    "Ctrl+Alt+P".to_string()
}

fn default_speak_speed() -> f32 {
    1.0
}

fn default_voice_da() -> String {
    "da_DK-talesyntese-medium".to_string()
}

fn default_voice_en() -> String {
    "en_US-lessac-medium".to_string()
}

impl Default for SpeakConfig {
    fn default() -> Self {
        SpeakConfig {
            enabled: true,
            read_da: default_speak_da(),
            read_en: default_speak_en(),
            stop: default_speak_stop(),
            pause: default_speak_pause(),
            speed: default_speak_speed(),
            voice_da: default_voice_da(),
            voice_en: default_voice_en(),
        }
    }
}

type SpeakShared = Arc<Mutex<SpeakConfig>>;

// ---------------------------------------------------------------- config

fn config_dir() -> PathBuf {
    let mut p = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    p.push("expander");
    let _ = fs::create_dir_all(&p);
    p
}

fn config_path() -> PathBuf {
    config_dir().join("triggers.json")
}

fn paused_flag_path() -> PathBuf {
    config_dir().join("paused")
}

fn default_entries() -> Vec<Entry> {
    vec![Entry {
        trigger: "omy".into(),
        text: "on my way".into(),
        space: true,
    }]
}

fn load_opt() -> Option<Vec<Entry>> {
    let s = fs::read_to_string(config_path()).ok()?;
    serde_json::from_str(&s).ok()
}

fn load() -> Vec<Entry> {
    load_opt().unwrap_or_else(default_entries)
}

fn save(entries: &[Entry]) {
    if let Ok(json) = serde_json::to_string_pretty(entries) {
        let _ = fs::write(config_path(), json);
    }
}

fn set_enabled(on: bool) {
    let p = paused_flag_path();
    if on {
        let _ = fs::remove_file(p);
    } else {
        let _ = fs::write(p, "paused");
    }
}

fn layout_path() -> PathBuf {
    config_dir().join("layout")
}

fn load_layout() -> String {
    fs::read_to_string(layout_path())
        .map(|s| s.trim().to_lowercase())
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "auto".to_string())
}

fn save_layout(v: &str) {
    let _ = fs::write(layout_path(), v);
}

// Interface language of the settings window
#[derive(Clone, Copy, PartialEq)]
enum Lang {
    En,
    Da,
}

fn tr(lang: Lang, en: &'static str, da: &'static str) -> &'static str {
    match lang {
        Lang::En => en,
        Lang::Da => da,
    }
}

fn lang_path() -> PathBuf {
    config_dir().join("language")
}

fn load_lang() -> Lang {
    let saved = fs::read_to_string(lang_path())
        .map(|s| s.trim().to_lowercase())
        .unwrap_or_default();
    match saved.as_str() {
        "da" => Lang::Da,
        "en" => Lang::En,
        // First run: follow the system language
        _ => {
            if std::env::var("LANG").map_or(false, |l| l.starts_with("da")) {
                Lang::Da
            } else {
                Lang::En
            }
        }
    }
}

fn save_lang(lang: Lang) {
    let _ = fs::write(
        lang_path(),
        match lang {
            Lang::En => "en",
            Lang::Da => "da",
        },
    );
}

fn voice_path() -> PathBuf {
    config_dir().join("voice.json")
}

fn load_voice_opt() -> Option<VoiceConfig> {
    let text = fs::read_to_string(voice_path()).ok()?;
    serde_json::from_str(&text).ok()
}

fn load_voice() -> VoiceConfig {
    load_voice_opt().unwrap_or_default()
}

fn save_voice(cfg: &VoiceConfig) {
    if let Ok(json) = serde_json::to_string_pretty(cfg) {
        let _ = fs::write(voice_path(), json);
    }
}

fn voice_mtime() -> Option<SystemTime> {
    fs::metadata(voice_path()).ok()?.modified().ok()
}

fn speak_path() -> PathBuf {
    config_dir().join("speak.json")
}

fn load_speak_opt() -> Option<SpeakConfig> {
    let text = fs::read_to_string(speak_path()).ok()?;
    serde_json::from_str(&text).ok()
}

fn load_speak() -> SpeakConfig {
    load_speak_opt().unwrap_or_default()
}

fn save_speak(cfg: &SpeakConfig) {
    if let Ok(json) = serde_json::to_string_pretty(cfg) {
        let _ = fs::write(speak_path(), json);
    }
}

fn speak_mtime() -> Option<SystemTime> {
    fs::metadata(speak_path()).ok()?.modified().ok()
}

fn ocr_path() -> PathBuf {
    config_dir().join("ocr.json")
}

fn load_ocr_opt() -> Option<OcrConfig> {
    let text = fs::read_to_string(ocr_path()).ok()?;
    serde_json::from_str(&text).ok()
}

fn load_ocr() -> OcrConfig {
    load_ocr_opt().unwrap_or_default()
}

fn save_ocr(cfg: &OcrConfig) {
    if let Ok(json) = serde_json::to_string_pretty(cfg) {
        let _ = fs::write(ocr_path(), json);
    }
}

fn ocr_mtime() -> Option<SystemTime> {
    fs::metadata(ocr_path()).ok()?.modified().ok()
}

fn mtime() -> Option<SystemTime> {
    fs::metadata(config_path()).ok()?.modified().ok()
}

// ---------------------------------------------------------------- background service

// PID file of the background service (used on Windows and macOS)
#[cfg(not(target_os = "linux"))]
fn pid_path() -> PathBuf {
    config_dir().join("daemon.pid")
}

#[cfg(not(target_os = "linux"))]
fn read_pid() -> Option<u32> {
    fs::read_to_string(pid_path()).ok()?.trim().parse().ok()
}

// ---- Linux: systemd user service, with setsid/pgrep/pkill as fallback

#[cfg(target_os = "linux")]
fn daemon_running() -> bool {
    Command::new("pgrep")
        .args(["-f", "expander --daemon"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn systemctl_user(action: &str) -> bool {
    Command::new("systemctl")
        .args(["--user", action, "expander.service"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn start_daemon() {
    // Preferred: the systemd user service (stops instantly at shutdown). Not used from an AppImage.
    if !setup::is_appimage() && systemctl_user("start") {
        return;
    }
    // From an AppImage the background copy is started from the AppImage file itself
    let exe = match setup::launcher_path() {
        Some(e) => e,
        None => return,
    };
    let mut cmd = Command::new("setsid");
    cmd.arg("--fork").arg(&exe).arg("--daemon").stdin(Stdio::null());
    match fs::File::create(config_dir().join("expander.log")) {
        Ok(f) => {
            if let Ok(f2) = f.try_clone() {
                cmd.stdout(f2);
            }
            cmd.stderr(f);
        }
        Err(_) => {
            cmd.stdout(Stdio::null());
            cmd.stderr(Stdio::null());
        }
    }
    let _ = cmd.spawn();
}

#[cfg(target_os = "linux")]
fn stop_daemon() {
    systemctl_user("stop");
    let _ = Command::new("pkill")
        .args(["-f", "expander --daemon"])
        .status();
}

// ---- Windows: hidden background process tracked by a PID file

#[cfg(windows)]
fn hidden(cmd: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000) // CREATE_NO_WINDOW
}

#[cfg(windows)]
fn daemon_running() -> bool {
    let pid = match read_pid() {
        Some(p) => p,
        None => return false,
    };
    let mut cmd = Command::new("tasklist");
    cmd.arg("/FI")
        .arg(format!("PID eq {}", pid))
        .arg("/FO")
        .arg("CSV")
        .arg("/NH");
    match hidden(&mut cmd).output() {
        Ok(o) => String::from_utf8_lossy(&o.stdout)
            .to_lowercase()
            .contains("expander.exe"),
        Err(_) => false,
    }
}

#[cfg(windows)]
fn start_daemon() {
    use std::os::windows::process::CommandExt;
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(_) => return,
    };
    let mut cmd = Command::new(exe);
    cmd.arg("--daemon").stdin(Stdio::null());
    match fs::File::create(config_dir().join("expander.log")) {
        Ok(f) => {
            if let Ok(f2) = f.try_clone() {
                cmd.stdout(f2);
            }
            cmd.stderr(f);
        }
        Err(_) => {
            cmd.stdout(Stdio::null());
            cmd.stderr(Stdio::null());
        }
    }
    // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP: survives closing the settings window
    let _ = cmd.creation_flags(0x0000_0008 | 0x0000_0200).spawn();
}

#[cfg(windows)]
fn stop_daemon() {
    if let Some(pid) = read_pid() {
        let mut cmd = Command::new("taskkill");
        cmd.arg("/PID").arg(pid.to_string()).arg("/F");
        let _ = hidden(&mut cmd).status();
    }
    let _ = fs::remove_file(pid_path());
}

// ---- Start when the user logs in (Windows: the "Run" list of the current user)

#[cfg(windows)]
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

#[cfg(windows)]
fn autostart_enabled() -> bool {
    let mut cmd = Command::new("reg");
    cmd.arg("query").arg(RUN_KEY).arg("/v").arg("TextExpander");
    hidden(&mut cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(windows)]
fn set_autostart(on: bool) {
    let mut cmd = Command::new("reg");
    if on {
        let exe = match std::env::current_exe() {
            Ok(e) => e,
            Err(_) => return,
        };
        cmd.arg("add")
            .arg(RUN_KEY)
            .arg("/v")
            .arg("TextExpander")
            .arg("/t")
            .arg("REG_SZ")
            .arg("/d")
            .arg(format!("\"{}\" --daemon", exe.display()))
            .arg("/f");
    } else {
        cmd.arg("delete")
            .arg(RUN_KEY)
            .arg("/v")
            .arg("TextExpander")
            .arg("/f");
    }
    let _ = hidden(&mut cmd).output();
}

#[cfg(not(windows))]
fn autostart_enabled() -> bool {
    false
}

#[cfg(not(windows))]
fn set_autostart(_on: bool) {}

// ---- Other Unix systems (macOS): detached process tracked by a PID file

#[cfg(not(any(target_os = "linux", windows)))]
fn daemon_running() -> bool {
    match read_pid() {
        Some(pid) => Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false),
        None => false,
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
fn start_daemon() {
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(_) => return,
    };
    let mut cmd = Command::new(exe);
    cmd.arg("--daemon").stdin(Stdio::null());
    match fs::File::create(config_dir().join("expander.log")) {
        Ok(f) => {
            if let Ok(f2) = f.try_clone() {
                cmd.stdout(f2);
            }
            cmd.stderr(f);
        }
        Err(_) => {
            cmd.stdout(Stdio::null());
            cmd.stderr(Stdio::null());
        }
    }
    let _ = cmd.spawn();
}

#[cfg(not(any(target_os = "linux", windows)))]
fn stop_daemon() {
    if let Some(pid) = read_pid() {
        let _ = Command::new("kill").arg(pid.to_string()).status();
    }
    let _ = fs::remove_file(pid_path());
}

fn reload_loop(
    entries: Shared,
    enabled: Arc<AtomicBool>,
    layout: LayoutSetting,
    ocr: OcrShared,
    voice: VoiceShared,
    speak: SpeakShared,
) -> ! {
    let mut last = mtime();
    let mut last_ocr = ocr_mtime();
    let mut last_voice = voice_mtime();
    let mut last_speak = speak_mtime();
    loop {
        thread::sleep(Duration::from_millis(1000));
        let now = mtime();
        if now != last {
            last = now;
            if let Some(new_entries) = load_opt() {
                *entries.lock().unwrap() = new_entries;
            }
        }
        let now_ocr = ocr_mtime();
        if now_ocr != last_ocr {
            last_ocr = now_ocr;
            if let Some(new_cfg) = load_ocr_opt() {
                *ocr.lock().unwrap() = new_cfg;
            }
        }
        let now_voice = voice_mtime();
        if now_voice != last_voice {
            last_voice = now_voice;
            if let Some(new_cfg) = load_voice_opt() {
                *voice.lock().unwrap() = new_cfg;
            }
        }
        let now_speak = speak_mtime();
        if now_speak != last_speak {
            last_speak = now_speak;
            if let Some(new_cfg) = load_speak_opt() {
                *speak.lock().unwrap() = new_cfg;
            }
        }
        enabled.store(!paused_flag_path().exists(), Ordering::SeqCst);
        let wanted = load_layout();
        let mut cur = layout.lock().unwrap();
        if *cur != wanted {
            *cur = wanted;
        }
    }
}

fn run_daemon() {
    #[cfg(not(target_os = "linux"))]
    {
        // Another background copy is already running: nothing to do
        if daemon_running() {
            return;
        }
        let _ = fs::write(pid_path(), std::process::id().to_string());
    }

    let entries: Shared = Arc::new(Mutex::new(load()));
    let enabled = Arc::new(AtomicBool::new(!paused_flag_path().exists()));
    let layout: LayoutSetting = Arc::new(Mutex::new(load_layout()));
    let ocr: OcrShared = Arc::new(Mutex::new(load_ocr()));
    let voice: VoiceShared = Arc::new(Mutex::new(load_voice()));
    let speak: SpeakShared = Arc::new(Mutex::new(load_speak()));

    #[cfg(target_os = "linux")]
    control::start(ocr.clone(), voice.clone(), speak.clone());

    // macOS: the key listener has to run on the main thread
    #[cfg(target_os = "macos")]
    {
        let (e, en, l, o, v, sp) = (
            entries.clone(),
            enabled.clone(),
            layout.clone(),
            ocr.clone(),
            voice.clone(),
            speak.clone(),
        );
        thread::spawn(move || reload_loop(e, en, l, o, v, sp));
        input::listen_blocking(entries, enabled, layout, ocr, voice, speak);
    }

    #[cfg(not(target_os = "macos"))]
    {
        input::start_listener(
            entries.clone(),
            enabled.clone(),
            layout.clone(),
            ocr.clone(),
            voice.clone(),
            speak.clone(),
        );
        reload_loop(entries, enabled, layout, ocr, voice, speak);
    }
}

// ---------------------------------------------------------------- input backends

#[cfg(not(target_os = "linux"))]
#[path = "input_other.rs"]
mod input;

#[cfg(target_os = "linux")]
mod speak;

#[cfg(target_os = "linux")]
mod control;

#[cfg(target_os = "linux")]
mod ocr;

#[cfg(target_os = "linux")]
mod setup;

#[cfg(target_os = "linux")]
mod voice;

#[cfg(target_os = "linux")]
#[path = "input_linux.rs"]
mod input;

// ---------------------------------------------------------------- settings window

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Expander,
    Ocr,
    Voice,
    Read,
    Setup,
}

struct App {
    entries: Vec<Entry>,
    enabled: bool,
    layout: String,
    lang: Lang,
    running: bool,
    last_check: Instant,
    new_trigger: String,
    new_text: String,
    new_space: bool,
    error: String,
    tab: Tab,
    ocr: OcrConfig,
    recording: bool,
    ocr_hint: String,
    ocr_status: Arc<Mutex<String>>,
    ocr_langs: Vec<String>,
    ocr_reqs: Vec<(String, bool)>,
    ocr_loaded: bool,
    voice: VoiceConfig,
    voice_recording: bool,
    voice_hint: String,
    voice_status: Arc<Mutex<String>>,
    voice_bench: Arc<Mutex<String>>,
    voice_reco: Arc<Mutex<Option<String>>>,
    speak: SpeakConfig,
    speak_recording: Option<usize>,
    speak_hint: String,
    speak_status: Arc<Mutex<String>>,
    autostart: bool,
    setup_status: Arc<Mutex<String>>,
    linux_autostart: bool,
    kbd_ok: bool,
}

// The command that starts the program from the desktop's own shortcut settings
#[cfg(target_os = "linux")]
fn expander_command() -> String {
    if let Some(p) = setup::appimage_path() {
        return format!("\"{}\"", p.display());
    }
    dirs::home_dir()
        .map(|h| h.join(".local/bin/expander").to_string_lossy().to_string())
        .unwrap_or_else(|| "expander".to_string())
}

// Name of a key that can be part of the OCR shortcut (letters, digits, F1-F12)
#[cfg(target_os = "linux")]
fn shortcut_key_name(key: egui::Key) -> Option<String> {
    let name = format!("{:?}", key);
    if name.len() == 1 && name.chars().all(|c| c.is_ascii_uppercase()) {
        return Some(name);
    }
    if let Some(d) = name.strip_prefix("Num") {
        if d.len() == 1 && d.chars().all(|c| c.is_ascii_digit()) {
            return Some(d.to_string());
        }
    }
    if let Some(n) = name.strip_prefix('F') {
        if let Ok(n) = n.parse::<u32>() {
            if (1..=12).contains(&n) {
                return Some(format!("F{}", n));
            }
        }
    }
    None
}

impl App {
    fn add(&mut self) {
        let trigger = self.new_trigger.trim().to_string();
        let text = self.new_text.trim().to_string();
        if trigger.is_empty() || text.is_empty() {
            self.error = tr(self.lang, "Fill in both fields.", "Udfyld begge felter.").into();
            return;
        }
        if trigger.contains(char::is_whitespace) {
            self.error = tr(
                self.lang,
                "A trigger can't contain spaces.",
                "En genvej må ikke indeholde mellemrum.",
            )
            .into();
            return;
        }
        if let Some(existing) = self
            .entries
            .iter_mut()
            .find(|e| e.trigger.to_lowercase() == trigger.to_lowercase())
        {
            existing.text = text;
            existing.space = self.new_space;
        } else {
            self.entries.push(Entry {
                trigger,
                text,
                space: self.new_space,
            });
        }
        save(&self.entries);
        self.new_trigger.clear();
        self.new_text.clear();
        self.error.clear();
    }

    // ---- tab 1: the text expander
    fn ui_expander(&mut self, ui: &mut egui::Ui, lang: Lang) {
        if ui
            .checkbox(&mut self.enabled, tr(lang, "Expansion enabled", "Udvidelse aktiveret"))
            .changed()
        {
            set_enabled(self.enabled);
        }

        // Keyboard layout (Linux only: Windows and macOS handle layouts themselves)
        if cfg!(target_os = "linux") {
            let layouts = [
                ("auto", "Auto (from KDE settings)", "Auto (fra KDE-indstillinger)"),
                ("dk", "Danish", "Dansk"),
                ("us", "English (US)", "Engelsk (US)"),
                ("uk", "English (UK)", "Engelsk (UK)"),
                ("fi", "Finnish / Swedish", "Finsk / Svensk"),
            ];
            let current_label = layouts
                .iter()
                .find(|&&(k, _, _)| k == self.layout)
                .map(|&(_, en, da)| tr(lang, en, da))
                .unwrap_or(tr(lang, "Auto (from KDE settings)", "Auto (fra KDE-indstillinger)"));
            ui.horizontal(|ui| {
                ui.label(tr(lang, "Keyboard layout:", "Tastaturlayout:"));
                egui::ComboBox::from_id_salt("layout")
                    .selected_text(current_label)
                    .show_ui(ui, |ui| {
                        for (code, en, da) in layouts {
                            if ui
                                .selectable_value(&mut self.layout, code.to_string(), tr(lang, en, da))
                                .changed()
                            {
                                save_layout(&self.layout);
                            }
                        }
                    });
            });
        }
        ui.separator();

        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.new_trigger)
                    .hint_text(tr(lang, "Trigger (e.g. omy)", "Genvej (fx omy)"))
                    .desired_width(120.0),
            );
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.new_text)
                    .hint_text(tr(lang, "Expands to (e.g. on my way)", "Bliver til (fx er på vej)"))
                    .desired_width(220.0),
            );
            ui.checkbox(&mut self.new_space, tr(lang, "Needs Space", "Kræver mellemrum"));
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button(tr(lang, "Add", "Tilføj")).clicked() || enter {
                self.add();
            }
        });
        ui.label(tr(
            lang,
            "Needs Space on: type the trigger, then press Space. Off: it expands instantly as soon as the trigger is typed.",
            "Kræver mellemrum slået til: skriv genvejen og tryk på mellemrum. Slået fra: den udvides med det samme, når genvejen er skrevet.",
        ));
        if !self.error.is_empty() {
            ui.colored_label(egui::Color32::LIGHT_RED, &self.error);
        }
        ui.separator();

        let mut remove: Option<usize> = None;
        let mut changed = false;
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("triggers")
                .striped(true)
                .num_columns(4)
                .spacing([16.0, 6.0])
                .show(ui, |ui| {
                    ui.strong(tr(lang, "Trigger", "Genvej"));
                    ui.strong(tr(lang, "Expands to", "Bliver til"));
                    ui.strong(tr(lang, "Needs Space", "Kræver mellemrum"));
                    ui.label("");
                    ui.end_row();
                    for (i, e) in self.entries.iter_mut().enumerate() {
                        ui.monospace(&e.trigger);
                        ui.label(&e.text);
                        if ui.checkbox(&mut e.space, "").changed() {
                            changed = true;
                        }
                        if ui.button(tr(lang, "Delete", "Slet")).clicked() {
                            remove = Some(i);
                        }
                        ui.end_row();
                    }
                });
        });
        if changed {
            save(&self.entries);
        }
        if let Some(i) = remove {
            self.entries.remove(i);
            save(&self.entries);
        }
    }

    // ---- tab 2: OCR (Linux)
    #[cfg(target_os = "linux")]
    fn refresh_ocr_info(&mut self) {
        self.ocr_langs = ocr::installed_languages();
        self.ocr_reqs = ocr::requirements();
        self.ocr_loaded = true;
    }

    #[cfg(target_os = "linux")]
    fn ui_ocr(&mut self, ui: &mut egui::Ui, lang: Lang) {
        if !self.ocr_loaded {
            self.refresh_ocr_info();
        }
        ui.label(tr(
            lang,
            "Press the shortcut, select an area of the screen, and the text in it is copied to the clipboard.",
            "Tryk på genvejen, markér et område på skærmen, og teksten i det kopieres til udklipsholderen.",
        ));
        ui.add_space(6.0);

        if ui
            .checkbox(&mut self.ocr.enabled, tr(lang, "OCR shortcut enabled", "OCR-genvej aktiveret"))
            .changed()
        {
            save_ocr(&self.ocr);
        }

        // Recording a new shortcut: the next key combination is taken
        if self.recording {
            let events = ui.input(|i| i.events.clone());
            for ev in events {
                if let egui::Event::Key { key, pressed: true, modifiers, .. } = ev {
                    if key == egui::Key::Escape {
                        self.recording = false;
                        break;
                    }
                    if let Some(name) = shortcut_key_name(key) {
                        if modifiers.ctrl || modifiers.alt || (name.starts_with('F') && name.len() > 1) {
                            let mut parts: Vec<String> = Vec::new();
                            if modifiers.ctrl {
                                parts.push("Ctrl".to_string());
                            }
                            if modifiers.alt {
                                parts.push("Alt".to_string());
                            }
                            if modifiers.shift {
                                parts.push("Shift".to_string());
                            }
                            parts.push(name);
                            self.ocr.shortcut = parts.join("+");
                            save_ocr(&self.ocr);
                            self.recording = false;
                            self.ocr_hint.clear();
                            break;
                        } else {
                            self.ocr_hint = tr(
                                lang,
                                "Use Ctrl or Alt together with a key, or a function key such as F9.",
                                "Brug Ctrl eller Alt sammen med en tast, eller en funktionstast som F9.",
                            )
                            .to_string();
                        }
                    }
                }
            }
        }

        ui.horizontal(|ui| {
            ui.label(tr(lang, "Shortcut:", "Genvej:"));
            if self.recording {
                ui.strong(tr(
                    lang,
                    "Press the new shortcut now (Esc cancels)",
                    "Tryk på den nye genvej nu (Esc annullerer)",
                ));
                if ui.button(tr(lang, "Cancel", "Annuller")).clicked() {
                    self.recording = false;
                }
            } else {
                ui.monospace(self.ocr.shortcut.clone());
                if ui.button(tr(lang, "Change...", "Skift...")).clicked() {
                    self.recording = true;
                    self.ocr_hint.clear();
                }
            }
        });
        if !self.ocr_hint.is_empty() {
            ui.colored_label(egui::Color32::LIGHT_RED, &self.ocr_hint);
        }
        ui.label(tr(
            lang,
            "If the shortcut types a letter into your document, use a key that types nothing (like F10), or let KDE handle the shortcut: System Settings > Keyboard > Shortcuts > Add Command, with this command:",
            "Hvis genvejen skriver et bogstav i dit dokument, så brug en tast, der intet skriver (fx F10), eller lad KDE styre genvejen: Systemindstillinger > Tastatur > Genveje > Tilføj kommando, med denne kommando:",
        ));
        ui.monospace(format!("{} --ocr", expander_command()));
        ui.label(tr(
            lang,
            "Pick a combination that no other program uses, for example Ctrl+Alt+O.",
            "Vælg en kombination, som intet andet program bruger, fx Ctrl+Alt+O.",
        ));
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            ui.label(tr(lang, "Text language(s):", "Tekstsprog:"));
            if ui
                .add(egui::TextEdit::singleline(&mut self.ocr.languages).desired_width(140.0))
                .changed()
            {
                save_ocr(&self.ocr);
            }
            ui.label(tr(lang, "e.g. eng+dan", "fx eng+dan"));
        });
        let langs_text = if self.ocr_langs.is_empty() {
            tr(lang, "none found", "ingen fundet").to_string()
        } else {
            self.ocr_langs.join(", ")
        };
        ui.label(format!(
            "{} {}",
            tr(lang, "Installed languages:", "Installerede sprog:"),
            langs_text
        ));
        ui.add_space(6.0);

        if ui.button(tr(lang, "Try OCR now", "Prøv OCR nu")).clicked() {
            let status = self.ocr_status.clone();
            let cfg = self.ocr.clone();
            *status.lock().unwrap() =
                tr(lang, "Select an area on the screen...", "Markér et område på skærmen...").to_string();
            thread::spawn(move || {
                let msg = match ocr::run_once(cfg) {
                    Ok(text) => {
                        let preview: String = text.chars().take(200).collect();
                        format!(
                            "{} {}: {}",
                            tr(lang, "Copied characters:", "Kopierede tegn:"),
                            text.chars().count(),
                            preview
                        )
                    }
                    Err(e) if e.is_empty() => tr(lang, "Cancelled.", "Annulleret.").to_string(),
                    Err(e) => format!("{} {}", tr(lang, "Error:", "Fejl:"), e),
                };
                *status.lock().unwrap() = msg;
            });
        }
        let status_text = self.ocr_status.lock().unwrap().clone();
        if !status_text.is_empty() {
            ui.label(status_text);
        }

        ui.separator();
        ui.horizontal(|ui| {
            ui.strong(tr(lang, "Requirements", "Krav"));
            if ui.button(tr(lang, "Check again", "Tjek igen")).clicked() {
                self.refresh_ocr_info();
            }
        });
        let mut missing = self.ocr_langs.is_empty();
        for (name, ok) in self.ocr_reqs.clone() {
            if ok {
                ui.colored_label(egui::Color32::LIGHT_GREEN, format!("OK: {}", name));
            } else {
                missing = true;
                ui.colored_label(egui::Color32::LIGHT_RED, format!("Missing: {}", name));
            }
        }
        if missing {
            ui.label(tr(lang, "Install on Fedora:", "Installér på Fedora:"));
            ui.monospace("sudo dnf install tesseract tesseract-langpack-dan wl-clipboard xclip");
        }
    }

    // ---- tab 3: voice typing (Linux)
    #[cfg(target_os = "linux")]
    fn ui_voice(&mut self, ui: &mut egui::Ui, lang: Lang) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.label(tr(
                lang,
                "Press the shortcut, speak, and the text is typed where your cursor is. Everything runs on this computer.",
                "Tryk på genvejen, tal, og teksten skrives, hvor din markør er. Alt kører på denne computer.",
            ));
            ui.add_space(6.0);

            if ui
                .checkbox(&mut self.voice.enabled, tr(lang, "Voice typing enabled", "Tale til tekst aktiveret"))
                .changed()
            {
                save_voice(&self.voice);
            }

            // Recording a new shortcut
            if self.voice_recording {
                let events = ui.input(|i| i.events.clone());
                for ev in events {
                    if let egui::Event::Key { key, pressed: true, modifiers, .. } = ev {
                        if key == egui::Key::Escape {
                            self.voice_recording = false;
                            break;
                        }
                        if let Some(name) = shortcut_key_name(key) {
                            if modifiers.ctrl || modifiers.alt || (name.starts_with('F') && name.len() > 1) {
                                let mut parts: Vec<String> = Vec::new();
                                if modifiers.ctrl {
                                    parts.push("Ctrl".to_string());
                                }
                                if modifiers.alt {
                                    parts.push("Alt".to_string());
                                }
                                if modifiers.shift {
                                    parts.push("Shift".to_string());
                                }
                                parts.push(name);
                                self.voice.shortcut = parts.join("+");
                                save_voice(&self.voice);
                                self.voice_recording = false;
                                self.voice_hint.clear();
                                break;
                            } else {
                                self.voice_hint = tr(
                                    lang,
                                    "Use Ctrl or Alt together with a key, or a function key such as F9.",
                                    "Brug Ctrl eller Alt sammen med en tast, eller en funktionstast som F9.",
                                )
                                .to_string();
                            }
                        }
                    }
                }
            }
            ui.horizontal(|ui| {
                ui.label(tr(lang, "Shortcut:", "Genvej:"));
                if self.voice_recording {
                    ui.strong(tr(
                        lang,
                        "Press the new shortcut now (Esc cancels)",
                        "Tryk på den nye genvej nu (Esc annullerer)",
                    ));
                    if ui.button(tr(lang, "Cancel", "Annuller")).clicked() {
                        self.voice_recording = false;
                    }
                } else {
                    ui.monospace(self.voice.shortcut.clone());
                    if ui.button(tr(lang, "Change...", "Skift...")).clicked() {
                        self.voice_recording = true;
                        self.voice_hint.clear();
                    }
                }
            });
            if !self.voice_hint.is_empty() {
                ui.colored_label(egui::Color32::LIGHT_RED, &self.voice_hint);
            }
            ui.label(tr(
                lang,
                "If the shortcut types a letter into your document, use a key that types nothing (like F9), or let KDE handle the shortcut: System Settings > Keyboard > Shortcuts > Add Command, with this command:",
                "Hvis genvejen skriver et bogstav i dit dokument, så brug en tast, der intet skriver (fx F9), eller lad KDE styre genvejen: Systemindstillinger > Tastatur > Genveje > Tilføj kommando, med denne kommando:",
            ));
            ui.monospace(format!("{} --voice-toggle", expander_command()));

            // Mode
            let modes = [
                ("toggle", "Press once to start, again to stop", "Tryk én gang for at starte, igen for at stoppe"),
                ("hold", "Hold the keys while speaking", "Hold tasterne nede, mens du taler"),
            ];
            let mode_label = modes
                .iter()
                .find(|&&(k, _, _)| k == self.voice.mode)
                .map(|&(_, en, da)| tr(lang, en, da))
                .unwrap_or("");
            ui.horizontal(|ui| {
                ui.label(tr(lang, "Mode:", "Tilstand:"));
                egui::ComboBox::from_id_salt("voice_mode")
                    .selected_text(mode_label)
                    .show_ui(ui, |ui| {
                        for (code, en, da) in modes {
                            if ui
                                .selectable_value(&mut self.voice.mode, code.to_string(), tr(lang, en, da))
                                .changed()
                            {
                                save_voice(&self.voice);
                            }
                        }
                    });
            });

            // Spoken language
            let languages = [
                ("da", "Danish", "Dansk"),
                ("en", "English", "Engelsk"),
                ("auto", "Detect automatically", "Find automatisk"),
            ];
            let language_label = languages
                .iter()
                .find(|&&(k, _, _)| k == self.voice.language)
                .map(|&(_, en, da)| tr(lang, en, da))
                .unwrap_or("");
            ui.horizontal(|ui| {
                ui.label(tr(lang, "Spoken language:", "Talt sprog:"));
                egui::ComboBox::from_id_salt("voice_language")
                    .selected_text(language_label)
                    .show_ui(ui, |ui| {
                        for (code, en, da) in languages {
                            if ui
                                .selectable_value(&mut self.voice.language, code.to_string(), tr(lang, en, da))
                                .changed()
                            {
                                save_voice(&self.voice);
                            }
                        }
                    });
            });

            ui.label(tr(
                lang,
                "The language setting is used by the Whisper models. Parakeet detects Danish and English by itself.",
                "Sprogvalget bruges af Whisper-modellerne. Parakeet finder selv ud af, om det er dansk eller engelsk.",
            ));

            // Live text while speaking
            ui.separator();
            if ui
                .checkbox(
                    &mut self.voice.live,
                    tr(
                        lang,
                        "Show what I say on the screen while I speak",
                        "Vis, hvad jeg siger, på skærmen, mens jeg taler",
                    ),
                )
                .changed()
            {
                save_voice(&self.voice);
            }
            if self.voice.live {
                let styles = [
                    ("box", "Box on the screen", "Boks på skærmen"),
                    ("notification", "Desktop notification (never takes focus)", "Skrivebordsbesked (tager aldrig fokus)"),
                ];
                let style_label = styles
                    .iter()
                    .find(|&&(k, _, _)| k == self.voice.live_style)
                    .map(|&(_, en, da)| tr(lang, en, da))
                    .unwrap_or("");
                ui.horizontal(|ui| {
                    ui.label(tr(lang, "Show it as:", "Vis som:"));
                    egui::ComboBox::from_id_salt("voice_style")
                        .selected_text(style_label)
                        .show_ui(ui, |ui| {
                            for (code, en, da) in styles {
                                if ui
                                    .selectable_value(&mut self.voice.live_style, code.to_string(), tr(lang, en, da))
                                    .changed()
                                {
                                    save_voice(&self.voice);
                                }
                            }
                        });
                });

                ui.label(tr(
                    lang,
                    "The text appears after each pause in your speech.",
                    "Teksten vises efter hver pause i din tale.",
                ));
            }

            // Behaviour
            ui.separator();
            ui.strong(tr(lang, "Behaviour", "Opførsel"));
            if ui
                .checkbox(
                    &mut self.voice.sounds,
                    tr(
                        lang,
                        "Play a sound when listening starts and stops",
                        "Afspil en lyd, når optagelsen starter og stopper",
                    ),
                )
                .changed()
            {
                save_voice(&self.voice);
            }
            if ui
                .button(tr(lang, "Play test sound", "Afspil testlyd"))
                .clicked()
            {
                let status = self.voice_status.clone();
                thread::spawn(move || {
                    let msg = match voice::test_sound() {
                        Ok(tool) => format!("{} {}.", tr(lang, "Sound played with", "Lyd afspillet med"), tool),
                        Err(e) => format!("{} {}", tr(lang, "Sound failed:", "Lyd fejlede:"), e),
                    };
                    *status.lock().unwrap() = msg;
                });
            }
            if ui
                .checkbox(
                    &mut self.voice.review,
                    tr(
                        lang,
                        "Show the text in a box I can correct before it is typed (Enter sends it)",
                        "Vis teksten i en boks, jeg kan rette i, før den skrives (Enter sender den)",
                    ),
                )
                .changed()
            {
                save_voice(&self.voice);
            }
            if ui
                .checkbox(
                    &mut self.voice.type_live,
                    tr(
                        lang,
                        "Type each phrase as soon as I pause (press once to start, again to stop)",
                        "Skriv hver sætning, så snart jeg holder pause (tryk én gang for at starte, igen for at stoppe)",
                    ),
                )
                .changed()
            {
                save_voice(&self.voice);
            }
            ui.horizontal(|ui| {
                ui.label(tr(
                    lang,
                    "A pause this long ends a phrase:",
                    "En pause af denne længde afslutter en sætning:",
                ));
                if ui
                    .add(egui::Slider::new(&mut self.voice.pause_ms, 250..=1500).suffix(" ms"))
                    .changed()
                {
                    save_voice(&self.voice);
                }
            });

            // Models
            ui.separator();
            ui.strong(tr(lang, "Models (download the ones you want to use)", "Modeller (hent dem, du vil bruge)"));
            ui.label(tr(
                lang,
                "Parakeet is the fastest choice. Bigger Whisper models can be more accurate but are much slower and need more memory. Pick the one that fits this computer.",
                "Parakeet er det hurtigste valg. Større Whisper-modeller kan være mere præcise, men er meget langsommere og bruger mere hukommelse. Vælg den, der passer til denne computer.",
            ));
            let dl = voice::download_snapshot();
            let mut action: Option<(&'static str, char)> = None;
            egui::Grid::new("voice_models")
                .striped(true)
                .num_columns(5)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    for m in voice::MODELS {
                        let ready = voice::model_ready(m.id);
                        let selected = self.voice.model == m.id;
                        if ui.radio(selected, "").clicked() && ready {
                            self.voice.model = m.id.to_string();
                            save_voice(&self.voice);
                        }
                        ui.label(tr(lang, m.label_en, m.label_da));
                        ui.label(format!("~{} MB", m.mb));
                        let downloading_this = matches!(&dl, Some((id, true, _)) if id == m.id);
                        if downloading_this {
                            let done = voice::download_bytes(m.id) as f64;
                            let pct = (done / (m.mb as f64 * 1_000_000.0) * 100.0).min(99.0);
                            ui.label(format!("{} {:.0}%", tr(lang, "Downloading", "Henter"), pct));
                            if ui.button(tr(lang, "Cancel", "Annuller")).clicked() {
                                action = Some((m.id, 'c'));
                            }
                        } else if ready {
                            ui.colored_label(egui::Color32::LIGHT_GREEN, tr(lang, "Downloaded", "Hentet"));
                            if ui.button(tr(lang, "Delete", "Slet")).clicked() {
                                action = Some((m.id, 'x'));
                            }
                        } else {
                            ui.label(tr(lang, "Not downloaded", "Ikke hentet"));
                            if ui.button(tr(lang, "Download", "Hent")).clicked() {
                                action = Some((m.id, 'd'));
                            }
                        }
                        ui.end_row();
                    }
                });
            match action {
                Some((id, 'd')) => voice::start_download(id),
                Some((_, 'c')) => voice::cancel_download(),
                Some((id, 'x')) => voice::delete_model(id),
                _ => {}
            }
            if let Some((id, false, err)) = &dl {
                if !err.is_empty() {
                    ui.colored_label(egui::Color32::LIGHT_RED, format!("{}: {}", id, err));
                }
            }
            ui.label(format!(
                "{} {}",
                tr(lang, "Models are stored in:", "Modellerne gemmes i:"),
                voice::models_dir().display()
            ));

            // Best model for this computer
            ui.separator();
            ui.strong(tr(lang, "Best model for this computer", "Bedste model til denne computer"));
            let hw = voice::hardware();
            ui.label(format!(
                "{}  |  {} {}  |  {} MB RAM",
                hw.cpu,
                hw.cores,
                tr(lang, "cores", "kerner"),
                hw.ram_mb
            ));
            let suggested = voice::suggested_model(&hw);
            let suggested_label = voice::MODELS
                .iter()
                .find(|m| m.id == suggested)
                .map(|m| tr(lang, m.label_en, m.label_da))
                .unwrap_or("");
            ui.label(format!(
                "{} {}",
                tr(lang, "Suggestion from the hardware:", "Forslag ud fra hardwaren:"),
                suggested_label
            ));
            ui.label(tr(
                lang,
                "For an exact answer: download a few models above, then run the speed test. It reads the same 5 seconds of your speech with every downloaded model and tells you which one is fast enough.",
                "For et præcist svar: hent et par modeller ovenfor og kør hastighedstesten. Den aflæser de samme 5 sekunders tale med hver hentet model og fortæller, hvilken der er hurtig nok.",
            ));
            if ui
                .button(tr(lang, "Run speed test (speak for 5 seconds)", "Kør hastighedstest (tal i 5 sekunder)"))
                .clicked()
            {
                let bench = self.voice_bench.clone();
                let reco = self.voice_reco.clone();
                let cfg = self.voice.clone();
                *bench.lock().unwrap() = tr(lang, "Starting...", "Starter...").to_string();
                *reco.lock().unwrap() = None;
                thread::spawn(move || match voice::benchmark(cfg, bench.clone()) {
                    Ok((text, best)) => {
                        *bench.lock().unwrap() = text;
                        *reco.lock().unwrap() = best;
                    }
                    Err(e) => {
                        *bench.lock().unwrap() = format!("{} {}", tr(lang, "Error:", "Fejl:"), e);
                    }
                });
            }
            let bench_text = self.voice_bench.lock().unwrap().clone();
            if !bench_text.is_empty() {
                ui.monospace(bench_text);
            }
            let best = self.voice_reco.lock().unwrap().clone();
            if let Some(id) = best {
                let label = voice::MODELS
                    .iter()
                    .find(|m| m.id == id)
                    .map(|m| tr(lang, m.label_en, m.label_da))
                    .unwrap_or("");
                ui.horizontal(|ui| {
                    ui.label(format!(
                        "{} {}",
                        tr(lang, "Best for this computer:", "Bedst til denne computer:"),
                        label
                    ));
                    if ui.button(tr(lang, "Use this model", "Brug denne model")).clicked() {
                        self.voice.model = id.clone();
                        save_voice(&self.voice);
                    }
                });
            }

            // Test
            ui.separator();
            if ui
                .button(tr(lang, "Test: record 5 seconds", "Test: optag 5 sekunder"))
                .clicked()
            {
                let status = self.voice_status.clone();
                let cfg = self.voice.clone();
                *status.lock().unwrap() = tr(
                    lang,
                    "Recording for 5 seconds - speak now...",
                    "Optager i 5 sekunder - tal nu...",
                )
                .to_string();
                thread::spawn(move || {
                    let msg = match voice::test_once(cfg, 5) {
                        Ok(t) if t.is_empty() => tr(lang, "No speech detected.", "Ingen tale registreret.").to_string(),
                        Ok(t) => format!("{} {}", tr(lang, "Heard:", "Hørte:"), t),
                        Err(e) => format!("{} {}", tr(lang, "Error:", "Fejl:"), e),
                    };
                    *status.lock().unwrap() = msg;
                });
            }
            let status_text = self.voice_status.lock().unwrap().clone();
            if !status_text.is_empty() {
                ui.label(status_text);
            }
            let stats_text = voice::read_stats();
            if !stats_text.is_empty() {
                ui.label(tr(
                    lang,
                    "Latest readings by the background service (how long each took):",
                    "Seneste aflæsninger fra baggrundstjenesten (hvor lang tid hver tog):",
                ));
                ui.monospace(stats_text);
            }

            // Requirements
            ui.separator();
            ui.strong(tr(lang, "Requirements", "Krav"));
            for (name, ok) in voice::requirements(&self.voice.model) {
                if ok {
                    ui.colored_label(egui::Color32::LIGHT_GREEN, format!("OK: {}", name));
                } else {
                    ui.colored_label(egui::Color32::LIGHT_RED, format!("Missing: {}", name));
                }
            }
        });
    }

    // ---- tab 4: read aloud (Linux)
    #[cfg(target_os = "linux")]
    fn ui_read(&mut self, ui: &mut egui::Ui, lang: Lang) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.label(tr(
                lang,
                "Select text in any program, press a shortcut, and it is read aloud. Everything runs on this computer.",
                "Markér tekst i et vilkårligt program, tryk på en genvej, og den læses højt. Alt kører på denne computer.",
            ));
            ui.add_space(6.0);

            if ui
                .checkbox(&mut self.speak.enabled, tr(lang, "Read-aloud shortcuts enabled", "Oplæsningsgenveje aktiveret"))
                .changed()
            {
                save_speak(&self.speak);
            }

            // Recording a new shortcut for one of the four actions
            if let Some(index) = self.speak_recording {
                let events = ui.input(|i| i.events.clone());
                for ev in events {
                    if let egui::Event::Key { key, pressed: true, modifiers, .. } = ev {
                        if key == egui::Key::Escape {
                            self.speak_recording = None;
                            break;
                        }
                        if let Some(name) = shortcut_key_name(key) {
                            if modifiers.ctrl || modifiers.alt || (name.starts_with('F') && name.len() > 1) {
                                let mut parts: Vec<String> = Vec::new();
                                if modifiers.ctrl {
                                    parts.push("Ctrl".to_string());
                                }
                                if modifiers.alt {
                                    parts.push("Alt".to_string());
                                }
                                if modifiers.shift {
                                    parts.push("Shift".to_string());
                                }
                                parts.push(name);
                                let combo = parts.join("+");
                                match index {
                                    0 => self.speak.read_da = combo,
                                    1 => self.speak.read_en = combo,
                                    2 => self.speak.stop = combo,
                                    _ => self.speak.pause = combo,
                                }
                                save_speak(&self.speak);
                                self.speak_recording = None;
                                self.speak_hint.clear();
                                break;
                            } else {
                                self.speak_hint = tr(
                                    lang,
                                    "Use Ctrl or Alt together with a key, or a function key such as F9.",
                                    "Brug Ctrl eller Alt sammen med en tast, eller en funktionstast som F9.",
                                )
                                .to_string();
                            }
                        }
                    }
                }
            }

            let rows = [
                (0usize, tr(lang, "Read the selected text in Danish:", "Læs den markerede tekst op på dansk:"), self.speak.read_da.clone()),
                (1usize, tr(lang, "Read the selected text in English:", "Læs den markerede tekst op på engelsk:"), self.speak.read_en.clone()),
                (2usize, tr(lang, "Stop reading:", "Stop oplæsning:"), self.speak.stop.clone()),
                (3usize, tr(lang, "Pause / resume:", "Pause / fortsæt:"), self.speak.pause.clone()),
            ];
            for (index, label, shortcut) in rows {
                ui.horizontal(|ui| {
                    ui.label(label);
                    if self.speak_recording == Some(index) {
                        ui.strong(tr(lang, "Press the new shortcut (Esc cancels)", "Tryk på den nye genvej (Esc annullerer)"));
                        if ui.button(tr(lang, "Cancel", "Annuller")).clicked() {
                            self.speak_recording = None;
                        }
                    } else {
                        ui.monospace(shortcut);
                        if ui.button(tr(lang, "Change...", "Skift...")).clicked() {
                            self.speak_recording = Some(index);
                            self.speak_hint.clear();
                        }
                    }
                });
            }
            if !self.speak_hint.is_empty() {
                ui.colored_label(egui::Color32::LIGHT_RED, &self.speak_hint);
            }

            ui.add_space(4.0);
            ui.label(tr(
                lang,
                "If a shortcut types a letter into your document, use a key that types nothing (like F6), or let KDE handle the shortcuts: System Settings > Keyboard > Shortcuts > Add Command, with these commands:",
                "Hvis en genvej skriver et bogstav i dit dokument, så brug en tast, der intet skriver (fx F6), eller lad KDE styre genvejene: Systemindstillinger > Tastatur > Genveje > Tilføj kommando, med disse kommandoer:",
            ));
            for flag in ["--read-da", "--read-en", "--read-stop", "--read-pause"] {
                ui.monospace(format!("{} {}", expander_command(), flag));
            }

            ui.separator();
            ui.horizontal(|ui| {
                ui.label(tr(lang, "Reading speed:", "Oplæsningshastighed:"));
                if ui
                    .add(egui::Slider::new(&mut self.speak.speed, 0.6..=1.6).suffix("x"))
                    .changed()
                {
                    save_speak(&self.speak);
                }
            });

            // Voices
            ui.separator();
            ui.strong(tr(lang, "Voices (download the ones you want)", "Stemmer (hent dem, du vil bruge)"));
            ui.label(tr(
                lang,
                "Choose one voice for Danish and one for English with the round buttons.",
                "Vælg én stemme til dansk og én til engelsk med de runde knapper.",
            ));
            let dl = speak::download_snapshot();
            let mut action: Option<(&'static str, char)> = None;
            egui::Grid::new("speak_voices")
                .striped(true)
                .num_columns(5)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    for v in speak::VOICES {
                        let ready = speak::voice_ready(v.id);
                        let selected = if v.lang == "da" {
                            self.speak.voice_da == v.id
                        } else {
                            self.speak.voice_en == v.id
                        };
                        if ui.radio(selected, "").clicked() && ready {
                            if v.lang == "da" {
                                self.speak.voice_da = v.id.to_string();
                            } else {
                                self.speak.voice_en = v.id.to_string();
                            }
                            save_speak(&self.speak);
                        }
                        ui.label(tr(lang, v.label_en, v.label_da));
                        ui.label(format!("~{} MB", v.mb));
                        let downloading_this = matches!(&dl, Some((id, true, _)) if id == v.id);
                        if downloading_this {
                            let done = speak::download_bytes(v.id) as f64;
                            let pct = (done / (v.mb as f64 * 1_000_000.0) * 100.0).min(99.0);
                            ui.label(format!("{} {:.0}%", tr(lang, "Downloading", "Henter"), pct));
                            if ui.button(tr(lang, "Cancel", "Annuller")).clicked() {
                                action = Some((v.id, 'c'));
                            }
                        } else if ready {
                            ui.colored_label(egui::Color32::LIGHT_GREEN, tr(lang, "Downloaded", "Hentet"));
                            if ui.button(tr(lang, "Delete", "Slet")).clicked() {
                                action = Some((v.id, 'x'));
                            }
                        } else {
                            ui.label(tr(lang, "Not downloaded", "Ikke hentet"));
                            if ui.button(tr(lang, "Download", "Hent")).clicked() {
                                action = Some((v.id, 'd'));
                            }
                        }
                        ui.end_row();
                    }
                });
            match action {
                Some((id, 'd')) => speak::start_download(id),
                Some((_, 'c')) => speak::cancel_download(),
                Some((id, 'x')) => speak::delete_voice(id),
                _ => {}
            }
            if let Some((id, false, err)) = &dl {
                if !err.is_empty() {
                    ui.colored_label(egui::Color32::LIGHT_RED, format!("{}: {}", id, err));
                }
            }

            // Test
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button(tr(lang, "Test Danish voice", "Test dansk stemme")).clicked() {
                    let status = self.speak_status.clone();
                    let cfg = self.speak.clone();
                    *status.lock().unwrap() = tr(lang, "Reading...", "Læser op...").to_string();
                    thread::spawn(move || {
                        let msg = match speak::say_test(cfg, "da") {
                            Ok(()) => String::new(),
                            Err(e) => format!("{} {}", tr(lang, "Error:", "Fejl:"), e),
                        };
                        *status.lock().unwrap() = msg;
                    });
                }
                if ui.button(tr(lang, "Test English voice", "Test engelsk stemme")).clicked() {
                    let status = self.speak_status.clone();
                    let cfg = self.speak.clone();
                    *status.lock().unwrap() = tr(lang, "Reading...", "Læser op...").to_string();
                    thread::spawn(move || {
                        let msg = match speak::say_test(cfg, "en") {
                            Ok(()) => String::new(),
                            Err(e) => format!("{} {}", tr(lang, "Error:", "Fejl:"), e),
                        };
                        *status.lock().unwrap() = msg;
                    });
                }
                if ui.button(tr(lang, "Stop", "Stop")).clicked() {
                    speak::stop();
                }
            });
            let status_text = self.speak_status.lock().unwrap().clone();
            if !status_text.is_empty() {
                ui.label(status_text);
            }

            // Requirements
            ui.separator();
            ui.strong(tr(lang, "Requirements", "Krav"));
            for (name, ok) in speak::requirements() {
                if ok {
                    ui.colored_label(egui::Color32::LIGHT_GREEN, format!("OK: {}", name));
                } else {
                    ui.colored_label(egui::Color32::LIGHT_RED, format!("Missing: {}", name));
                }
            }
        });
    }

    // ---- tab 5: setup (Linux)
    #[cfg(target_os = "linux")]
    fn ui_setup(&mut self, ui: &mut egui::Ui, lang: Lang) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.label(tr(
                lang,
                "A few one-time steps so that everything works on this computer.",
                "Et par engangstrin, så alt virker på denne computer.",
            ));
            if let Some(path) = setup::appimage_path() {
                ui.label(format!("{} {}", tr(lang, "Running from:", "Kører fra:"), path.display()));
            }
            ui.add_space(6.0);

            // 1. keyboard access
            ui.strong(tr(lang, "1. Keyboard access", "1. Adgang til tastaturet"));
            if self.kbd_ok {
                ui.colored_label(
                    egui::Color32::LIGHT_GREEN,
                    tr(
                        lang,
                        "OK: the program can see shortcuts and type text.",
                        "OK: programmet kan se genveje og skrive tekst.",
                    ),
                );
            } else {
                ui.colored_label(
                    egui::Color32::LIGHT_RED,
                    tr(
                        lang,
                        "Not set up yet. This asks for your password once, and you must log out and back in afterwards.",
                        "Ikke sat op endnu. Det beder om din adgangskode én gang, og du skal bagefter logge ud og ind igen.",
                    ),
                );
                if ui
                    .button(tr(lang, "Set up keyboard access...", "Sæt adgang til tastaturet op..."))
                    .clicked()
                {
                    let status = self.setup_status.clone();
                    thread::spawn(move || {
                        let msg = match setup::grant_keyboard_access() {
                            Ok(()) => tr(
                                lang,
                                "Done. Now log out and back in once.",
                                "Færdig. Log nu ud og ind igen én gang.",
                            )
                            .to_string(),
                            Err(e) => format!("{} {}", tr(lang, "Error:", "Fejl:"), e),
                        };
                        *status.lock().unwrap() = msg;
                    });
                }
            }
            ui.add_space(8.0);

            // 2. start at login
            ui.strong(tr(lang, "2. Start at login", "2. Start ved login"));
            if setup::is_appimage() {
                if ui
                    .checkbox(
                        &mut self.linux_autostart,
                        tr(
                            lang,
                            "Start the background service when I log in",
                            "Start baggrundstjenesten, når jeg logger ind",
                        ),
                    )
                    .changed()
                {
                    if let Err(e) = setup::set_autostart(self.linux_autostart) {
                        *self.setup_status.lock().unwrap() =
                            format!("{} {}", tr(lang, "Error:", "Fejl:"), e);
                    }
                }
            } else {
                ui.label(tr(
                    lang,
                    "Handled by the installer (a systemd user service).",
                    "Håndteres af installationsprogrammet (en systemd-brugertjeneste).",
                ));
            }
            ui.add_space(8.0);

            // 3. application menu
            ui.strong(tr(lang, "3. Application menu", "3. Programmenu"));
            if setup::is_appimage() {
                if ui
                    .button(tr(
                        lang,
                        "Add Text Expander to the application menu",
                        "Tilføj Text Expander til programmenuen",
                    ))
                    .clicked()
                {
                    let msg = match setup::install_menu_entries() {
                        Ok(()) => tr(lang, "Added to the application menu.", "Tilføjet til programmenuen.").to_string(),
                        Err(e) => format!("{} {}", tr(lang, "Error:", "Fejl:"), e),
                    };
                    *self.setup_status.lock().unwrap() = msg;
                }
                ui.label(tr(
                    lang,
                    "This also adds entries you can pick when you set shortcuts in your desktop's settings.",
                    "Det tilføjer også punkter, du kan vælge, når du sætter genveje op i dit skrivebords indstillinger.",
                ));
            } else {
                ui.label(tr(lang, "Added by the installer.", "Tilføjet af installationsprogrammet."));
            }

            let status_text = self.setup_status.lock().unwrap().clone();
            if !status_text.is_empty() {
                ui.add_space(6.0);
                ui.label(status_text);
            }

            // 4. helpers that come from the desktop
            ui.add_space(8.0);
            ui.separator();
            ui.strong(tr(lang, "4. Helpers from your desktop", "4. Hjælpere fra dit skrivebord"));
            ui.label(tr(
                lang,
                "These are not part of the program. Without them some features are limited.",
                "De er ikke en del af programmet. Uden dem er nogle funktioner begrænsede.",
            ));
            for (name, ok) in ocr::requirements() {
                if ok {
                    ui.colored_label(egui::Color32::LIGHT_GREEN, format!("OK: {}", name));
                } else {
                    ui.colored_label(egui::Color32::LIGHT_RED, format!("Missing: {}", name));
                }
            }
            let notify_ok = ocr::have("notify-send");
            if notify_ok {
                ui.colored_label(egui::Color32::LIGHT_GREEN, "OK: Notifications (notify-send)");
            } else {
                ui.colored_label(egui::Color32::LIGHT_RED, "Missing: Notifications (notify-send), optional");
            }
        });
    }

    // OCR is only available on Linux for now
    #[cfg(not(target_os = "linux"))]
    fn ui_ocr(&mut self, _ui: &mut egui::Ui, _lang: Lang) {}

    #[cfg(not(target_os = "linux"))]
    fn ui_voice(&mut self, _ui: &mut egui::Ui, _lang: Lang) {}

    #[cfg(not(target_os = "linux"))]
    fn ui_read(&mut self, _ui: &mut egui::Ui, _lang: Lang) {}

    #[cfg(not(target_os = "linux"))]
    fn ui_setup(&mut self, _ui: &mut egui::Ui, _lang: Lang) {}
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let lang = self.lang;
        // faster refresh while a model is downloading (progress)
        #[cfg(target_os = "linux")]
        if matches!(voice::download_snapshot(), Some((_, true, _)))
            || matches!(speak::download_snapshot(), Some((_, true, _)))
        {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        if self.last_check.elapsed() > Duration::from_secs(2) {
            self.running = daemon_running();
            #[cfg(target_os = "linux")]
            {
                self.kbd_ok = setup::keyboard_access_ok();
            }
            self.last_check = Instant::now();
        }
        ctx.request_repaint_after(Duration::from_secs(2));

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading(format!("Text Expander  v{}", env!("CARGO_PKG_VERSION")));

            ui.horizontal(|ui| {
                if self.running {
                    ui.colored_label(
                        egui::Color32::LIGHT_GREEN,
                        tr(lang, "Running in the background", "Kører i baggrunden"),
                    );
                    if ui.button(tr(lang, "Stop", "Stop")).clicked() {
                        stop_daemon();
                        self.running = false;
                    }
                } else {
                    ui.colored_label(egui::Color32::LIGHT_RED, tr(lang, "Not running", "Kører ikke"));
                    if ui.button(tr(lang, "Start", "Start")).clicked() {
                        start_daemon();
                        thread::sleep(Duration::from_millis(400));
                        self.running = daemon_running();
                    }
                }
            });
            ui.label(tr(
                lang,
                "You can close this window; everything keeps working in the background.",
                "Du kan lukke dette vindue; alt kører videre i baggrunden.",
            ));

            // Interface language
            ui.horizontal(|ui| {
                ui.label(tr(lang, "Language:", "Sprog:"));
                egui::ComboBox::from_id_salt("lang")
                    .selected_text(if lang == Lang::Da { "Dansk" } else { "English" })
                    .show_ui(ui, |ui| {
                        if ui.selectable_value(&mut self.lang, Lang::En, "English").changed() {
                            save_lang(self.lang);
                        }
                        if ui.selectable_value(&mut self.lang, Lang::Da, "Dansk").changed() {
                            save_lang(self.lang);
                        }
                    });
            });

            // Start at login (Windows; on Linux the installer sets up the background service)
            if cfg!(windows) {
                if ui
                    .checkbox(
                        &mut self.autostart,
                        tr(
                            lang,
                            "Start automatically when I log in",
                            "Start automatisk, når jeg logger ind",
                        ),
                    )
                    .changed()
                {
                    set_autostart(self.autostart);
                }
            }

            // Tabs (the OCR tab exists on Linux only for now)
            if cfg!(target_os = "linux") {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.tab, Tab::Expander, tr(lang, "Text expander", "Tekstudvider"));
                    ui.selectable_value(&mut self.tab, Tab::Ocr, tr(lang, "OCR (text from screen)", "OCR (tekst fra skærm)"));
                    ui.selectable_value(&mut self.tab, Tab::Voice, tr(lang, "Voice typing", "Tale til tekst"));
                    ui.selectable_value(&mut self.tab, Tab::Read, tr(lang, "Read aloud", "Oplæsning"));
                    ui.selectable_value(&mut self.tab, Tab::Setup, tr(lang, "Setup", "Opsætning"));
                });
            }
            ui.separator();

            match self.tab {
                Tab::Expander => self.ui_expander(ui, lang),
                Tab::Ocr => self.ui_ocr(ui, lang),
                Tab::Voice => self.ui_voice(ui, lang),
                Tab::Read => self.ui_read(ui, lang),
                Tab::Setup => self.ui_setup(ui, lang),
            }
        });
    }
}

// ---------------------------------------------------------------- live text box (voice typing)

#[cfg(target_os = "linux")]
const OVERLAY_W: f32 = 760.0;
#[cfg(target_os = "linux")]
const OVERLAY_H: f32 = 175.0;

#[cfg(target_os = "linux")]
struct Overlay {
    positioned: bool,
}

#[cfg(target_os = "linux")]
impl eframe::App for Overlay {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(100));
        // Bottom centre of the screen
        if !self.positioned {
            if let Some(monitor) = ctx.input(|i| i.viewport().monitor_size) {
                let pos = egui::pos2(
                    (monitor.x - OVERLAY_W) / 2.0,
                    monitor.y - OVERLAY_H - 90.0,
                );
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
                self.positioned = true;
            }
        }
        // The voice typing asked the box to close
        if config_dir().join("voice_close").exists() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        let text = fs::read_to_string(config_dir().join("voice_live.txt")).unwrap_or_default();
        let level: f32 = fs::read_to_string(config_dir().join("voice_level.txt"))
            .ok()
            .and_then(|t| t.trim().parse().ok())
            .unwrap_or(0.0);
        let note = fs::read_to_string(config_dir().join("voice_note.txt")).unwrap_or_default();
        let frame = egui::Frame::none()
            .fill(egui::Color32::from_rgba_unmultiplied(25, 25, 25, 235))
            .rounding(12.0)
            .inner_margin(14.0);
        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.colored_label(egui::Color32::LIGHT_RED, "Listening...");
                ui.add(egui::ProgressBar::new(level.clamp(0.0, 1.0)).desired_width(220.0));
            });
            let shown = if text.trim().is_empty() {
                "...".to_string()
            } else {
                text
            };
            ui.label(egui::RichText::new(shown).size(22.0).color(egui::Color32::WHITE));
            if !note.trim().is_empty() {
                ui.label(egui::RichText::new(note.trim()).size(13.0).color(egui::Color32::GRAY));
            }
        });
    }
}

#[cfg(target_os = "linux")]
fn run_overlay() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([OVERLAY_W, OVERLAY_H])
            .with_decorations(false)
            .with_always_on_top()
            .with_transparent(true)
            .with_mouse_passthrough(true)
            .with_active(false)
            // a notification-type window: the desktop does not give it the keyboard focus
            .with_window_type(egui::X11WindowType::Notification)
            .with_title("Text Expander voice"),
        ..Default::default()
    };
    eframe::run_native(
        "Text Expander voice",
        options,
        Box::new(|_cc| Ok(Box::new(Overlay { positioned: false }))),
    )
}

// ---------------------------------------------------------------- edit box (voice typing)

#[cfg(target_os = "linux")]
struct Review {
    text: String,
    lang: Lang,
    first_frame: bool,
}

#[cfg(target_os = "linux")]
impl eframe::App for Review {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let lang = self.lang;
        if self.first_frame {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }

        // Enter sends, Shift+Enter makes a new line, Esc cancels
        let mut send = false;
        let mut cancel = false;
        ctx.input_mut(|i| {
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
                send = true;
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                cancel = true;
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label(tr(
                lang,
                "Correct the text if needed. Enter = send it to the text field, Shift+Enter = new line, Esc = cancel.",
                "Ret teksten, hvis der er fejl. Enter = send den til tekstfeltet, Shift+Enter = ny linje, Esc = annuller.",
            ));
            let edit = egui::TextEdit::multiline(&mut self.text)
                .desired_width(f32::INFINITY)
                .desired_rows(5)
                .font(egui::FontId::proportional(20.0));
            let response = ui.add(edit);
            if self.first_frame {
                response.request_focus();
            }
            ui.horizontal(|ui| {
                if ui.button(tr(lang, "Send", "Send")).clicked() {
                    send = true;
                }
                if ui.button(tr(lang, "Cancel", "Annuller")).clicked() {
                    cancel = true;
                }
            });
        });
        self.first_frame = false;

        if send {
            let _ = fs::write(config_dir().join("voice_review_out.txt"), &self.text);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if cancel {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

#[cfg(target_os = "linux")]
fn run_review() -> eframe::Result<()> {
    let text = fs::read_to_string(config_dir().join("voice_review_in.txt")).unwrap_or_default();
    let lang = load_lang();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([780.0, 300.0])
            .with_always_on_top()
            .with_title(tr(lang, "Text Expander - correct and send", "Text Expander - ret og send")),
        ..Default::default()
    };
    eframe::run_native(
        "Text Expander edit",
        options,
        Box::new(move |_cc| {
            Ok(Box::new(Review {
                text,
                lang,
                first_frame: true,
            }))
        }),
    )
}

#[cfg(target_os = "linux")]
fn keyboard_ok_now() -> bool {
    setup::keyboard_access_ok()
}

#[cfg(not(target_os = "linux"))]
fn keyboard_ok_now() -> bool {
    true
}

#[cfg(target_os = "linux")]
fn linux_autostart_now() -> bool {
    setup::autostart_enabled()
}

#[cfg(not(target_os = "linux"))]
fn linux_autostart_now() -> bool {
    false
}

fn main() -> eframe::Result<()> {
    // Commands for the desktop's own shortcuts: expander --voice-toggle / --voice-start / --voice-stop / --ocr
    #[cfg(target_os = "linux")]
    {
        let args: Vec<String> = std::env::args().collect();
        for (flag, command) in [
            ("--voice-toggle", "voice-toggle"),
            ("--voice-start", "voice-start"),
            ("--voice-stop", "voice-stop"),
            ("--ocr", "ocr"),
            ("--read-da", "read-da"),
            ("--read-en", "read-en"),
            ("--read-stop", "read-stop"),
            ("--read-pause", "read-pause"),
        ] {
            if args.iter().any(|a| a == flag) {
                if !control::send(command) {
                    // the background service is not running: start it and try again
                    start_daemon();
                    thread::sleep(Duration::from_secs(3));
                    control::send(command);
                }
                return Ok(());
            }
        }
    }

    #[cfg(target_os = "linux")]
    if std::env::args().any(|a| a == "--overlay") {
        return run_overlay();
    }

    #[cfg(target_os = "linux")]
    if std::env::args().any(|a| a == "--review") {
        return run_review();
    }

    if std::env::args().any(|a| a == "--daemon") {
        run_daemon();
        return Ok(());
    }

    // Settings window: make sure the background service is running first
    if !daemon_running() {
        start_daemon();
        thread::sleep(Duration::from_millis(400));
    }

    let kbd_ok = keyboard_ok_now();
    let app = App {
        entries: load(),
        enabled: !paused_flag_path().exists(),
        layout: load_layout(),
        lang: load_lang(),
        running: daemon_running(),
        last_check: Instant::now(),
        new_trigger: String::new(),
        new_text: String::new(),
        new_space: true,
        error: String::new(),
        tab: if kbd_ok { Tab::Expander } else { Tab::Setup },
        ocr: load_ocr(),
        recording: false,
        ocr_hint: String::new(),
        ocr_status: Arc::new(Mutex::new(String::new())),
        ocr_langs: Vec::new(),
        ocr_reqs: Vec::new(),
        ocr_loaded: false,
        voice: load_voice(),
        voice_recording: false,
        voice_hint: String::new(),
        voice_status: Arc::new(Mutex::new(String::new())),
        voice_bench: Arc::new(Mutex::new(String::new())),
        voice_reco: Arc::new(Mutex::new(None)),
        speak: load_speak(),
        speak_recording: None,
        speak_hint: String::new(),
        speak_status: Arc::new(Mutex::new(String::new())),
        autostart: autostart_enabled(),
        setup_status: Arc::new(Mutex::new(String::new())),
        linux_autostart: linux_autostart_now(),
        kbd_ok,
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([780.0, 740.0]),
        ..Default::default()
    };
    eframe::run_native("Text Expander", options, Box::new(|_cc| Ok(Box::new(app))))
}
