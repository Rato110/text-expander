use super::ocr::{gui_cmd, have, notify};
use super::SpeakConfig;
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

// ------------------------------------------------------------ voices

pub struct Voice {
    pub id: &'static str,
    pub lang: &'static str,
    pub label_en: &'static str,
    pub label_da: &'static str,
    pub path: &'static str,
    pub mb: u32,
}

pub const VOICES: &[Voice] = &[
    Voice {
        id: "da_DK-talesyntese-medium",
        lang: "da",
        label_en: "Danish - Talesyntese",
        label_da: "Dansk - Talesyntese",
        path: "da/da_DK/talesyntese/medium",
        mb: 63,
    },
    Voice {
        id: "en_US-lessac-medium",
        lang: "en",
        label_en: "English (US) - Lessac",
        label_da: "Engelsk (US) - Lessac",
        path: "en/en_US/lessac/medium",
        mb: 63,
    },
    Voice {
        id: "en_US-amy-medium",
        lang: "en",
        label_en: "English (US) - Amy",
        label_da: "Engelsk (US) - Amy",
        path: "en/en_US/amy/medium",
        mb: 63,
    },
    Voice {
        id: "en_US-ryan-medium",
        lang: "en",
        label_en: "English (US) - Ryan",
        label_da: "Engelsk (US) - Ryan",
        path: "en/en_US/ryan/medium",
        mb: 63,
    },
    Voice {
        id: "en_GB-alan-medium",
        lang: "en",
        label_en: "English (UK) - Alan",
        label_da: "Engelsk (UK) - Alan",
        path: "en/en_GB/alan/medium",
        mb: 63,
    },
    Voice {
        id: "en_GB-jenny_dioco-medium",
        lang: "en",
        label_en: "English (UK) - Jenny",
        label_da: "Engelsk (UK) - Jenny",
        path: "en/en_GB/jenny_dioco/medium",
        mb: 63,
    },
];

fn data_dir() -> PathBuf {
    let mut p = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    p.push("expander");
    p
}

pub fn voices_dir() -> PathBuf {
    data_dir().join("voices")
}

fn voice_model(id: &str) -> PathBuf {
    voices_dir().join(format!("{}.onnx", id))
}

pub fn voice_ready(id: &str) -> bool {
    voice_model(id).is_file() && voices_dir().join(format!("{}.onnx.json", id)).is_file()
}

pub struct VoiceDownload {
    pub id: String,
    pub running: bool,
    pub error: String,
    pub pid: u32,
    pub cancelled: bool,
}

static DOWNLOAD: Mutex<Option<VoiceDownload>> = Mutex::new(None);

// (voice id, still running, error text)
pub fn download_snapshot() -> Option<(String, bool, String)> {
    DOWNLOAD
        .lock()
        .unwrap()
        .as_ref()
        .map(|d| (d.id.clone(), d.running, d.error.clone()))
}

pub fn download_bytes(id: &str) -> u64 {
    fs::metadata(voices_dir().join(format!("{}.onnx.part", id)))
        .map(|m| m.len())
        .unwrap_or(0)
}

fn finish_download(id: &str, result: Result<(), String>) {
    let mut st = DOWNLOAD.lock().unwrap();
    *st = Some(VoiceDownload {
        id: id.to_string(),
        running: false,
        error: result.err().unwrap_or_default(),
        pid: 0,
        cancelled: false,
    });
}

pub fn start_download(id: &str) {
    let voice = match VOICES.iter().find(|v| v.id == id) {
        Some(v) => v,
        None => return,
    };
    {
        let mut st = DOWNLOAD.lock().unwrap();
        if st.as_ref().map_or(false, |d| d.running) {
            return; // one download at a time
        }
        *st = Some(VoiceDownload {
            id: id.to_string(),
            running: true,
            error: String::new(),
            pid: 0,
            cancelled: false,
        });
    }
    let id = id.to_string();
    let path = voice.path;
    thread::spawn(move || {
        let _ = fs::create_dir_all(voices_dir());
        // the small settings file first, then the voice itself
        for ext in ["onnx.json", "onnx"] {
            let url = format!(
                "https://huggingface.co/rhasspy/piper-voices/resolve/main/{}/{}.{}",
                path, id, ext
            );
            let part = voices_dir().join(format!("{}.{}.part", id, ext));
            let done = voices_dir().join(format!("{}.{}", id, ext));
            let spawned = Command::new("curl")
                .args(["-L", "--fail", "--silent", "--show-error", "-C", "-", "-o"])
                .arg(&part)
                .arg(&url)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn();
            let mut child = match spawned {
                Ok(c) => c,
                Err(e) => {
                    finish_download(&id, Err(format!("Could not start curl: {}", e)));
                    return;
                }
            };
            if let Some(st) = DOWNLOAD.lock().unwrap().as_mut() {
                st.pid = child.id();
            }
            let mut err_text = String::new();
            if let Some(mut stderr) = child.stderr.take() {
                let _ = stderr.read_to_string(&mut err_text);
            }
            let status = child.wait();
            let cancelled = DOWNLOAD
                .lock()
                .unwrap()
                .as_ref()
                .map_or(false, |d| d.cancelled);
            match status {
                Ok(s) if s.success() => {
                    if let Err(e) = fs::rename(&part, &done) {
                        finish_download(&id, Err(e.to_string()));
                        return;
                    }
                }
                _ if cancelled => {
                    finish_download(&id, Err(String::new()));
                    return;
                }
                _ => {
                    let msg = if err_text.trim().is_empty() {
                        "Download failed.".to_string()
                    } else {
                        err_text.trim().to_string()
                    };
                    finish_download(&id, Err(msg));
                    return;
                }
            }
        }
        finish_download(&id, Ok(()));
    });
}

pub fn cancel_download() {
    let pid = {
        let mut st = DOWNLOAD.lock().unwrap();
        match st.as_mut() {
            Some(d) if d.running => {
                d.cancelled = true;
                d.pid
            }
            _ => 0,
        }
    };
    if pid != 0 {
        let _ = Command::new("kill").arg(pid.to_string()).status();
    }
}

pub fn delete_voice(id: &str) {
    for ext in ["onnx", "onnx.json", "onnx.part", "onnx.json.part"] {
        let _ = fs::remove_file(voices_dir().join(format!("{}.{}", id, ext)));
    }
}

// ------------------------------------------------------------ the voice engine (Piper in a Python helper)

// Request: one line of JSON. Reply: "OK" or "ERR ..." (the sound is written to a WAV file).
const TTS_SCRIPT: &str = r#"import json
import sys
import wave

from piper import PiperVoice

try:
    from piper import SynthesisConfig
except Exception:
    SynthesisConfig = None

voices = {}


def load_voice(path):
    if path not in voices:
        voices[path] = PiperVoice.load(path)
    return voices[path]


out = sys.stdout
out.write("READY\n")
out.flush()

for raw in sys.stdin:
    raw = raw.strip()
    if not raw:
        continue
    try:
        request = json.loads(raw)
        voice = load_voice(request["model"])
        scale = float(request.get("length_scale", 1.0))
        with wave.open(request["out"], "wb") as wav:
            if hasattr(voice, "synthesize_wav"):
                if SynthesisConfig is not None:
                    voice.synthesize_wav(request["text"], wav, syn_config=SynthesisConfig(length_scale=scale))
                else:
                    voice.synthesize_wav(request["text"], wav)
            else:
                voice.synthesize(request["text"], wav, length_scale=scale)
        out.write("OK\n")
    except Exception as error:
        out.write("ERR " + " ".join(repr(error).split()) + "\n")
    out.flush()
"#;

struct Tts {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    last_used: Instant,
}

static TTS: Mutex<Option<Tts>> = Mutex::new(None);
static TTS_STARTING: AtomicBool = AtomicBool::new(false);

fn python_bin() -> PathBuf {
    // inside the AppImage the Python is part of the bundle
    if let Some(p) = std::env::var_os("EXPANDER_PYTHON") {
        return PathBuf::from(p);
    }
    data_dir().join("py").join("bin").join("python")
}

// Is the "piper" package inside the private Python?
pub fn piper_installed() -> bool {
    if std::env::var_os("EXPANDER_BUNDLED").is_some() {
        return true; // the AppImage contains the voice engine
    }
    let lib = data_dir().join("py").join("lib");
    match fs::read_dir(lib) {
        Ok(entries) => entries
            .flatten()
            .any(|e| e.path().join("site-packages").join("piper").is_dir()),
        Err(_) => false,
    }
}

fn start_tts() -> Result<(), String> {
    if !python_bin().is_file() || !piper_installed() {
        return Err("The reading voice engine (Piper) is not installed. Run the installer again.".to_string());
    }
    fs::create_dir_all(data_dir()).map_err(|e| format!("Could not create the data folder: {}", e))?;
    let script = data_dir().join("tts_server.py");
    fs::write(&script, TTS_SCRIPT).map_err(|e| format!("Could not write the helper: {}", e))?;
    let log = fs::File::create(super::config_dir().join("tts-server.log"))
        .map_err(|e| format!("Could not create the log file: {}", e))?;
    let mut child = Command::new(python_bin())
        .arg("-u")
        .arg(&script)
        .env("PYTHONUNBUFFERED", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(log))
        .spawn()
        .map_err(|e| format!("Could not start the reading voice engine: {}", e))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Could not talk to the reading voice engine.".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Could not read from the reading voice engine.".to_string())?;
    let mut reader = BufReader::new(stdout);
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).unwrap_or(0);
        if n == 0 {
            let _ = child.kill();
            let _ = child.wait();
            return Err("The reading voice engine did not start. See tts-server.log in the settings folder.".to_string());
        }
        if line.trim() == "READY" {
            break;
        }
    }
    *TTS.lock().unwrap() = Some(Tts {
        child,
        stdin,
        stdout: reader,
        last_used: Instant::now(),
    });
    Ok(())
}

fn ensure_tts() -> Result<(), String> {
    let began = Instant::now();
    loop {
        if TTS.lock().unwrap().is_some() {
            return Ok(());
        }
        if !TTS_STARTING.swap(true, Ordering::SeqCst) {
            let result = start_tts();
            TTS_STARTING.store(false, Ordering::SeqCst);
            return result;
        }
        if began.elapsed() > Duration::from_secs(120) {
            return Err("The reading voice engine took too long to start.".to_string());
        }
        thread::sleep(Duration::from_millis(100));
    }
}

// Frees the memory when nothing was read for ten minutes
pub fn start_idle_watch() {
    thread::spawn(|| loop {
        thread::sleep(Duration::from_secs(30));
        let mut guard = TTS.lock().unwrap();
        let idle = guard
            .as_ref()
            .map_or(false, |t| t.last_used.elapsed() > Duration::from_secs(600));
        if idle && GENERATION_ACTIVE.load(Ordering::SeqCst) == 0 {
            if let Some(mut t) = guard.take() {
                let _ = t.child.kill();
                let _ = t.child.wait();
            }
        }
    });
}

// Turns one piece of text into a WAV file
fn synth_request(model: &Path, text: &str, out: &Path, length_scale: f32) -> Result<(), String> {
    ensure_tts()?;
    let request = serde_json::json!({
        "model": model.to_string_lossy(),
        "text": text,
        "out": out.to_string_lossy(),
        "length_scale": length_scale,
    })
    .to_string();
    let mut guard = TTS.lock().unwrap();
    let reply: Option<String> = match guard.as_mut() {
        Some(t) => {
            let outcome: std::io::Result<String> = (|| -> std::io::Result<String> {
                t.stdin.write_all(request.as_bytes())?;
                t.stdin.write_all(b"\n")?;
                t.stdin.flush()?;
                let mut line = String::new();
                t.stdout.read_line(&mut line)?;
                Ok(line)
            })();
            match outcome {
                Ok(line) if !line.is_empty() => {
                    t.last_used = Instant::now();
                    Some(line)
                }
                _ => None,
            }
        }
        None => None,
    };
    match reply {
        Some(line) => {
            let t = line.trim();
            if t == "OK" {
                Ok(())
            } else {
                Err(format!("The reading voice engine reported: {}", t.trim_start_matches("ERR ")))
            }
        }
        None => {
            if let Some(mut dead) = guard.take() {
                let _ = dead.child.kill();
                let _ = dead.child.wait();
            }
            Err("The reading voice engine stopped. Try again.".to_string())
        }
    }
}

// ------------------------------------------------------------ getting the selected text

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

// ('some text',)  ->  some text
fn parse_gvariant_string(output: &str) -> Option<String> {
    let t = output.trim();
    let t = t.strip_prefix('(')?;
    let t = t.strip_suffix(",)")?;
    let quote = t.chars().next()?;
    if (quote != '\'' && quote != '"') || t.len() < 2 || !t.ends_with(quote) {
        return None;
    }
    let inner = &t[1..t.len() - 1];
    let mut result = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('t') => result.push('\t'),
                Some('r') => result.push('\r'),
                Some('\\') => result.push('\\'),
                Some('\'') => result.push('\''),
                Some('"') => result.push('"'),
                Some(other) => {
                    result.push('\\');
                    result.push(other);
                }
                None => result.push('\\'),
            }
        } else {
            result.push(c);
        }
    }
    Some(result)
}

fn klipper_call(method: &str, arg: Option<String>) -> Option<String> {
    let mut cmd = gui_cmd("gdbus");
    cmd.args([
        "call",
        "--session",
        "--dest",
        "org.kde.klipper",
        "--object-path",
        "/klipper",
        "--method",
    ])
    .arg(format!("org.kde.klipper.klipper.{}", method));
    if let Some(a) = arg {
        cmd.arg(a);
    }
    let output = cmd
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        None
    }
}

fn klipper_get() -> Option<String> {
    klipper_call("getClipboardContents", None).and_then(|o| parse_gvariant_string(&o))
}

fn klipper_set(text: &str) {
    let _ = klipper_call("setClipboardContents", Some(gvariant_quote(text)));
}

fn klipper_clear() {
    let _ = klipper_call("clearClipboardContents", None);
}

fn read_clipboard_other() -> Option<String> {
    if have("wl-paste") {
        if let Ok(o) = gui_cmd("wl-paste")
            .arg("--no-newline")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
        {
            if o.status.success() {
                return Some(String::from_utf8_lossy(&o.stdout).to_string());
            }
        }
    }
    if have("xclip") {
        if let Ok(o) = gui_cmd("xclip")
            .args(["-o", "-selection", "clipboard"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
        {
            if o.status.success() {
                return Some(String::from_utf8_lossy(&o.stdout).to_string());
            }
        }
    }
    None
}

// Copies the text that is selected in the program in front (the same as pressing Ctrl+C)
fn grab_selection() -> Result<String, String> {
    if klipper_get().is_some() {
        // KDE: clear the clipboard first, so that "nothing selected" can be told from "the same text"
        let old = klipper_get().unwrap_or_default();
        klipper_clear();
        if !super::input::press_copy() {
            return Err("The typing keyboard is not ready, so the text could not be copied.".to_string());
        }
        let began = Instant::now();
        let mut text = String::new();
        while began.elapsed() < Duration::from_millis(1500) {
            thread::sleep(Duration::from_millis(80));
            text = klipper_get().unwrap_or_default();
            if !text.trim().is_empty() {
                break;
            }
        }
        if text.trim().is_empty() {
            if !old.is_empty() {
                klipper_set(&old);
            }
            return Err("No text is selected (or the program did not copy it).".to_string());
        }
        Ok(text)
    } else {
        if !super::input::press_copy() {
            return Err("The typing keyboard is not ready, so the text could not be copied.".to_string());
        }
        thread::sleep(Duration::from_millis(350));
        match read_clipboard_other() {
            Some(t) if !t.trim().is_empty() => Ok(t),
            _ => Err("No text could be copied. On KDE this needs Klipper; elsewhere wl-clipboard or xclip.".to_string()),
        }
    }
}

// ------------------------------------------------------------ reading it out loud

static GENERATION: AtomicU32 = AtomicU32::new(0);
static GENERATION_ACTIVE: AtomicU32 = AtomicU32::new(0);
static PLAYER_PID: AtomicU32 = AtomicU32::new(0);
static PAUSED: AtomicBool = AtomicBool::new(false);

fn signal(pid: u32, name: &str) {
    if pid != 0 {
        let _ = Command::new("kill")
            .arg(format!("-{}", name))
            .arg(pid.to_string())
            .status();
    }
}

fn kill_player() {
    let pid = PLAYER_PID.load(Ordering::SeqCst);
    signal(pid, "KILL");
}

// Plays one WAV file and waits until it is finished (or stopped)
fn play_file(path: &Path) {
    let file = path.to_string_lossy().to_string();
    for (tool, args) in [
        ("paplay", Vec::<&str>::new()),
        ("pw-play", Vec::<&str>::new()),
        ("pw-cat", vec!["--playback"]),
        ("aplay", vec!["-q"]),
    ] {
        if !have(tool) {
            continue;
        }
        let spawned = gui_cmd(tool)
            .args(&args)
            .arg(&file)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        if let Ok(mut child) = spawned {
            PLAYER_PID.store(child.id(), Ordering::SeqCst);
            let status = child.wait();
            PLAYER_PID.store(0, Ordering::SeqCst);
            if status.map(|s| s.success()).unwrap_or(false) {
                return;
            }
            // stopped on purpose, or this player failed: do not start another one then
            return;
        }
    }
}

// Cuts a text into pieces at the ends of sentences
fn split_text(text: &str, max: usize) -> Vec<String> {
    let mut sentences: Vec<String> = Vec::new();
    let mut current = String::new();
    for c in text.replace('\r', "").chars() {
        current.push(c);
        if c == '.' || c == '!' || c == '?' || c == '\n' {
            let t = current.trim().to_string();
            if !t.is_empty() {
                sentences.push(t);
            }
            current.clear();
        }
    }
    let rest = current.trim().to_string();
    if !rest.is_empty() {
        sentences.push(rest);
    }

    let mut chunks: Vec<String> = Vec::new();
    let mut acc = String::new();
    for sentence in sentences {
        // a very long sentence is cut at spaces
        let mut parts: Vec<String> = Vec::new();
        if sentence.chars().count() > max {
            let mut piece = String::new();
            for word in sentence.split_whitespace() {
                if !piece.is_empty() && piece.chars().count() + word.chars().count() + 1 > max {
                    parts.push(piece.clone());
                    piece.clear();
                }
                if !piece.is_empty() {
                    piece.push(' ');
                }
                piece.push_str(word);
            }
            if !piece.is_empty() {
                parts.push(piece);
            }
        } else {
            parts.push(sentence);
        }
        for p in parts {
            if !acc.is_empty() && acc.chars().count() + p.chars().count() + 1 > max {
                chunks.push(acc.clone());
                acc.clear();
            }
            if !acc.is_empty() {
                acc.push(' ');
            }
            acc.push_str(&p);
        }
    }
    if !acc.is_empty() {
        chunks.push(acc);
    }
    chunks
}

// The next piece is prepared while the current one is playing
fn read_text(text: &str, voice_id: &str, speed: f32, generation: u32) {
    let chunks = split_text(text, 280);
    if chunks.is_empty() {
        return;
    }
    GENERATION_ACTIVE.fetch_add(1, Ordering::SeqCst);
    let model = voice_model(voice_id);
    let length_scale = 1.0 / speed.clamp(0.5, 2.0);
    let (tx, rx) = mpsc::sync_channel::<PathBuf>(2);
    let synth = thread::spawn(move || {
        for (i, chunk) in chunks.iter().enumerate() {
            if GENERATION.load(Ordering::SeqCst) != generation {
                break;
            }
            let out = std::env::temp_dir().join(format!(
                "expander-tts-{}-{}-{}.wav",
                std::process::id(),
                generation,
                i
            ));
            match synth_request(&model, chunk, &out, length_scale) {
                Ok(()) => {
                    if tx.send(out).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    notify("Read aloud failed", &e);
                    break;
                }
            }
        }
    });
    for path in rx {
        // while paused, wait here between two pieces
        while PAUSED.load(Ordering::SeqCst) && GENERATION.load(Ordering::SeqCst) == generation {
            thread::sleep(Duration::from_millis(60));
        }
        if GENERATION.load(Ordering::SeqCst) == generation {
            play_file(&path);
        }
        let _ = fs::remove_file(&path);
    }
    let _ = synth.join();
    GENERATION_ACTIVE.fetch_sub(1, Ordering::SeqCst);
}

pub fn stop() {
    GENERATION.fetch_add(1, Ordering::SeqCst);
    PAUSED.store(false, Ordering::SeqCst);
    kill_player();
}

pub fn toggle_pause() {
    let now_paused = !PAUSED.load(Ordering::SeqCst);
    PAUSED.store(now_paused, Ordering::SeqCst);
    let pid = PLAYER_PID.load(Ordering::SeqCst);
    signal(pid, if now_paused { "STOP" } else { "CONT" });
}

// Selected text -> spoken
fn read_selection(cfg: SpeakConfig, lang: &str) {
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    PAUSED.store(false, Ordering::SeqCst);
    kill_player();
    let voice_id = if lang == "da" {
        cfg.voice_da.clone()
    } else {
        cfg.voice_en.clone()
    };
    if !voice_ready(&voice_id) {
        notify(
            "Read aloud",
            "Download a voice first: open Text Expander > Read aloud.",
        );
        return;
    }
    let text = match grab_selection() {
        Ok(t) => t,
        Err(e) => {
            notify("Read aloud", &e);
            return;
        }
    };
    if GENERATION.load(Ordering::SeqCst) != generation {
        return; // stopped or replaced meanwhile
    }
    read_text(&text, &voice_id, cfg.speed, generation);
}

pub fn run_action(action: &str, cfg: SpeakConfig) {
    match action {
        "read-da" => read_selection(cfg, "da"),
        "read-en" => read_selection(cfg, "en"),
        "read-stop" => stop(),
        "read-pause" => toggle_pause(),
        _ => {}
    }
}

// The "Test" buttons in the window
pub fn say_test(cfg: SpeakConfig, lang: &str) -> Result<(), String> {
    let voice_id = if lang == "da" {
        cfg.voice_da.clone()
    } else {
        cfg.voice_en.clone()
    };
    if !voice_ready(&voice_id) {
        return Err("Download this voice first.".to_string());
    }
    let sample = if lang == "da" {
        "Hej! Det her er en test af oplæsningen."
    } else {
        "Hello! This is a test of the reading voice."
    };
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    PAUSED.store(false, Ordering::SeqCst);
    kill_player();
    read_text(sample, &voice_id, cfg.speed, generation);
    Ok(())
}

pub fn requirements() -> Vec<(String, bool)> {
    vec![
        (
            "Reading voice engine (Piper in Python, set up by the installer)".to_string(),
            piper_installed(),
        ),
        (
            "Sound player (paplay, pw-play or aplay)".to_string(),
            have("paplay") || have("pw-play") || have("pw-cat") || have("aplay"),
        ),
        (
            "KDE clipboard (gdbus + Klipper) to copy the selected text".to_string(),
            have("gdbus"),
        ),
    ]
}
