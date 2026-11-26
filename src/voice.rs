use super::ocr::{gui_cmd, have, notify};
use super::VoiceConfig;
use std::{
    collections::VecDeque,
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

// ------------------------------------------------------------ models

#[derive(Clone, Copy, PartialEq)]
pub enum Engine {
    Whisper,
    Parakeet,
    // Parakeet in a Python helper that keeps the model loaded in memory (fast)
    ParakeetOnnx,
}

pub struct Model {
    pub id: &'static str,
    pub label_en: &'static str,
    pub label_da: &'static str,
    pub file: &'static str,
    pub repo: &'static str,
    pub engine: Engine,
    pub mb: u32,
}

pub const MODELS: &[Model] = &[
    Model {
        id: "parakeet-onnx",
        label_en: "Parakeet - fastest, stays loaded in memory (recommended)",
        label_da: "Parakeet - hurtigst, bliver i hukommelsen (anbefalet)",
        file: "",
        repo: "",
        engine: Engine::ParakeetOnnx,
        mb: 670,
    },
    Model {
        id: "parakeet-q8_0",
        label_en: "Parakeet - fast, Danish and English (recommended)",
        label_da: "Parakeet - hurtig, dansk og engelsk (anbefalet)",
        file: "ggml-parakeet-tdt-0.6b-v3-q8_0.bin",
        repo: "ggml-org/parakeet-GGUF",
        engine: Engine::Parakeet,
        mb: 669,
    },
    Model {
        id: "parakeet-q4_k",
        label_en: "Parakeet, compressed - fast, a bit smaller",
        label_da: "Parakeet, komprimeret - hurtig, lidt mindre",
        file: "ggml-parakeet-tdt-0.6b-v3-q4_k.bin",
        repo: "ggml-org/parakeet-GGUF",
        engine: Engine::Parakeet,
        mb: 416,
    },
    Model {
        id: "parakeet-q4_0",
        label_en: "Parakeet, smallest - for weaker computers",
        label_da: "Parakeet, mindst - til svagere computere",
        file: "ggml-parakeet-tdt-0.6b-v3-q4_0.bin",
        repo: "ggml-org/parakeet-GGUF",
        engine: Engine::Parakeet,
        mb: 356,
    },
    Model {
        id: "tiny-q5_1",
        label_en: "Whisper Tiny - fastest Whisper, weak at Danish",
        label_da: "Whisper Tiny - hurtigst, svag til dansk",
        file: "ggml-tiny-q5_1.bin",
        repo: "ggerganov/whisper.cpp",
        engine: Engine::Whisper,
        mb: 31,
    },
    Model {
        id: "base-q5_1",
        label_en: "Whisper Base - very fast, so-so Danish",
        label_da: "Whisper Base - meget hurtig, middel dansk",
        file: "ggml-base-q5_1.bin",
        repo: "ggerganov/whisper.cpp",
        engine: Engine::Whisper,
        mb: 57,
    },
    Model {
        id: "small-q5_1",
        label_en: "Whisper Small - fast, fair Danish",
        label_da: "Whisper Small - hurtig, nogenlunde dansk",
        file: "ggml-small-q5_1.bin",
        repo: "ggerganov/whisper.cpp",
        engine: Engine::Whisper,
        mb: 181,
    },
    Model {
        id: "medium-q5_0",
        label_en: "Whisper Medium - good Danish, slow",
        label_da: "Whisper Medium - godt dansk, langsom",
        file: "ggml-medium-q5_0.bin",
        repo: "ggerganov/whisper.cpp",
        engine: Engine::Whisper,
        mb: 514,
    },
    Model {
        id: "large-v3-turbo-q5_0",
        label_en: "Whisper Large v3 Turbo, compressed - very good, slow",
        label_da: "Whisper Large v3 Turbo, komprimeret - meget god, langsom",
        file: "ggml-large-v3-turbo-q5_0.bin",
        repo: "ggerganov/whisper.cpp",
        engine: Engine::Whisper,
        mb: 547,
    },
    Model {
        id: "large-v3-turbo",
        label_en: "Whisper Large v3 Turbo - very good, needs more memory",
        label_da: "Whisper Large v3 Turbo - meget god, bruger mere hukommelse",
        file: "ggml-large-v3-turbo.bin",
        repo: "ggerganov/whisper.cpp",
        engine: Engine::Whisper,
        mb: 1624,
    },
    Model {
        id: "large-v3-q5_0",
        label_en: "Whisper Large v3, compressed - best quality, very slow",
        label_da: "Whisper Large v3, komprimeret - bedste kvalitet, meget langsom",
        file: "ggml-large-v3-q5_0.bin",
        repo: "ggerganov/whisper.cpp",
        engine: Engine::Whisper,
        mb: 1080,
    },
    Model {
        id: "large-v3",
        label_en: "Whisper Large v3 - best quality, slowest",
        label_da: "Whisper Large v3 - bedste kvalitet, langsomst",
        file: "ggml-large-v3.bin",
        repo: "ggerganov/whisper.cpp",
        engine: Engine::Whisper,
        mb: 3100,
    },
];

fn data_dir() -> PathBuf {
    let mut p = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    p.push("expander");
    p
}

pub fn models_dir() -> PathBuf {
    data_dir().join("models")
}

fn live_path() -> PathBuf {
    super::config_dir().join("voice_live.txt")
}

pub fn model_file(id: &str) -> Option<PathBuf> {
    MODELS
        .iter()
        .find(|m| m.id == id)
        .map(|m| models_dir().join(m.file))
}

pub fn model_engine(id: &str) -> Option<Engine> {
    MODELS.iter().find(|m| m.id == id).map(|m| m.engine)
}

// A model is only renamed to its final name when the download completed
pub fn model_ready(id: &str) -> bool {
    match model_engine(id) {
        Some(Engine::ParakeetOnnx) => onnx_marker().is_file() && python_bin().is_file(),
        Some(_) => model_file(id).map(|p| p.is_file()).unwrap_or(false),
        None => false,
    }
}

pub struct DownloadState {
    pub id: String,
    pub running: bool,
    pub error: String,
    pub pid: u32,
    pub cancelled: bool,
}

static DOWNLOAD: Mutex<Option<DownloadState>> = Mutex::new(None);

// (model id, still running, error text)
pub fn download_snapshot() -> Option<(String, bool, String)> {
    DOWNLOAD
        .lock()
        .unwrap()
        .as_ref()
        .map(|d| (d.id.clone(), d.running, d.error.clone()))
}

pub fn download_bytes(id: &str) -> u64 {
    if model_engine(id) == Some(Engine::ParakeetOnnx) {
        return dir_size(&onnx_cache_dir());
    }
    match MODELS.iter().find(|m| m.id == id) {
        Some(m) => fs::metadata(models_dir().join(format!("{}.part", m.file)))
            .map(|md| md.len())
            .unwrap_or(0),
        None => 0,
    }
}

fn finish_download(id: &str, result: Result<(), String>) {
    let mut st = DOWNLOAD.lock().unwrap();
    *st = Some(DownloadState {
        id: id.to_string(),
        running: false,
        error: result.err().unwrap_or_default(),
        pid: 0,
        cancelled: false,
    });
}

pub fn start_download(id: &str) {
    let model = match MODELS.iter().find(|m| m.id == id) {
        Some(m) => m,
        None => return,
    };
    {
        let mut st = DOWNLOAD.lock().unwrap();
        if st.as_ref().map_or(false, |d| d.running) {
            return; // one download at a time
        }
        *st = Some(DownloadState {
            id: id.to_string(),
            running: true,
            error: String::new(),
            pid: 0,
            cancelled: false,
        });
    }
    if model.engine == Engine::ParakeetOnnx {
        // The helper downloads the model itself the first time it starts
        let onnx_id = id.to_string();
        thread::spawn(move || {
            let result = ensure_server();
            let cancelled = DOWNLOAD
                .lock()
                .unwrap()
                .as_ref()
                .map_or(false, |d| d.cancelled);
            match result {
                Ok(()) => finish_download(&onnx_id, Ok(())),
                Err(_) if cancelled => finish_download(&onnx_id, Err(String::new())),
                Err(e) => finish_download(&onnx_id, Err(e)),
            }
        });
        return;
    }
    let id = id.to_string();
    let file = model.file;
    let repo = model.repo;
    thread::spawn(move || {
        let _ = fs::create_dir_all(models_dir());
        let part = models_dir().join(format!("{}.part", file));
        let done = models_dir().join(file);
        let url = format!("https://huggingface.co/{}/resolve/main/{}", repo, file);
        // curl can resume an interrupted download (-C -)
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
            Ok(s) if s.success() => match fs::rename(&part, &done) {
                Ok(_) => finish_download(&id, Ok(())),
                Err(e) => finish_download(&id, Err(e.to_string())),
            },
            _ if cancelled => finish_download(&id, Err(String::new())),
            _ => {
                let msg = if err_text.trim().is_empty() {
                    "Download failed.".to_string()
                } else {
                    err_text.trim().to_string()
                };
                finish_download(&id, Err(msg));
            }
        }
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

pub fn delete_model(id: &str) {
    if model_engine(id) == Some(Engine::ParakeetOnnx) {
        stop_server();
        let _ = fs::remove_file(onnx_marker());
        let _ = fs::remove_dir_all(onnx_cache_dir());
        return;
    }
    if let Some(m) = MODELS.iter().find(|m| m.id == id) {
        let _ = fs::remove_file(models_dir().join(m.file));
        let _ = fs::remove_file(models_dir().join(format!("{}.part", m.file)));
    }
}

// ------------------------------------------------------------ tools

// whisper-cli (Whisper models) or parakeet-cli (Parakeet models), built by the installer
fn engine_binary(engine: Engine) -> Option<PathBuf> {
    let names: &[&str] = match engine {
        Engine::Whisper => &["whisper-cli", "whisper-cpp"],
        Engine::Parakeet => &["parakeet-cli"],
        Engine::ParakeetOnnx => &[],
    };
    for name in names {
        let local = data_dir().join("whisper").join(name);
        if local.is_file() {
            return Some(local);
        }
    }
    for name in names {
        if have(name) {
            return Some(PathBuf::from(*name));
        }
    }
    None
}

fn missing_engine_message(engine: Engine) -> String {
    match engine {
        Engine::Whisper => "The Whisper engine (whisper-cli) is not installed. Run the installer again.".to_string(),
        Engine::Parakeet => "The Parakeet engine (parakeet-cli) is not installed. Run the installer again.".to_string(),
        Engine::ParakeetOnnx => "The fast Parakeet engine (Python packages) is not installed. Run the installer again.".to_string(),
    }
}

fn recorder_tool() -> Option<&'static str> {
    if have("parec") {
        Some("parec")
    } else if have("arecord") {
        Some("arecord")
    } else {
        None
    }
}

pub fn requirements(model: &str) -> Vec<(String, bool)> {
    let engine = model_engine(model).unwrap_or(Engine::Parakeet);
    let engine_label = match engine {
        Engine::Whisper => "Voice engine for Whisper models (whisper-cli, built by the installer)",
        Engine::Parakeet => "Voice engine for Parakeet models (parakeet-cli, built by the installer)",
        Engine::ParakeetOnnx => "Fast Parakeet engine (Python + onnx-asr, set up by the installer)",
    };
    vec![
        (engine_label.to_string(), engine_available(engine)),
        (
            "Recording tool (parec from pulseaudio-utils, or arecord)".to_string(),
            recorder_tool().is_some(),
        ),
        (
            "Sound player for the start/stop beeps (paplay, pw-play or aplay)".to_string(),
            sound_player_available(),
        ),
        (
            "A downloaded model (see the list above)".to_string(),
            model_ready(model),
        ),
    ]
}

// ------------------------------------------------------------ the fast Parakeet engine

// A small Python program that loads the Parakeet model once and then reads phrases from us.
// Request: 4 bytes (length) + 16 kHz mono 16-bit audio. Reply: one line of text.
const SERVER_SCRIPT: &str = r#"import struct
import sys

import numpy as np
import onnx_asr


def log(message):
    sys.stderr.write(str(message) + "\n")
    sys.stderr.flush()


model = onnx_asr.load_model("nemo-parakeet-tdt-0.6b-v3", quantization="int8")
try:
    model.recognize(np.zeros(16000, dtype=np.float32), sample_rate=16000)
except Exception as error:
    log("warm-up failed: %r" % (error,))

out = sys.stdout.buffer
inp = sys.stdin.buffer
out.write(b"READY\n")
out.flush()


def read_exact(count):
    data = b""
    while len(data) < count:
        chunk = inp.read(count - len(data))
        if not chunk:
            return None
        data += chunk
    return data


while True:
    header = read_exact(4)
    if header is None:
        break
    (size,) = struct.unpack("<I", header)
    payload = read_exact(size) if size else b""
    if payload is None:
        break
    try:
        audio = np.frombuffer(payload, dtype="<i2").astype(np.float32) / 32768.0
        text = model.recognize(audio, sample_rate=16000)
        if not isinstance(text, str):
            text = str(text)
        reply = " ".join(text.split())
    except Exception as error:
        reply = "ERR " + " ".join(repr(error).split())
    out.write(reply.encode("utf-8") + b"\n")
    out.flush()
"#;

struct Server {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    last_used: Instant,
}

static SERVER: Mutex<Option<Server>> = Mutex::new(None);
static SERVER_STARTING: AtomicBool = AtomicBool::new(false);

fn python_bin() -> PathBuf {
    // inside the AppImage the Python is part of the bundle
    if let Some(p) = std::env::var_os("EXPANDER_PYTHON") {
        return PathBuf::from(p);
    }
    data_dir().join("py").join("bin").join("python")
}

fn onnx_marker() -> PathBuf {
    data_dir().join("parakeet-onnx.ready")
}

fn onnx_cache_dir() -> PathBuf {
    data_dir()
        .join("hf")
        .join("hub")
        .join("models--istupakov--parakeet-tdt-0.6b-v3-onnx")
}

fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    if let Ok(entries) = fs::read_dir(path) {
        for e in entries.flatten() {
            if let Ok(md) = e.metadata() {
                if md.is_dir() {
                    total += dir_size(&e.path());
                } else {
                    total += md.len();
                }
            }
        }
    }
    total
}

fn engine_available(engine: Engine) -> bool {
    match engine {
        Engine::ParakeetOnnx => python_bin().is_file(),
        _ => engine_binary(engine).is_some(),
    }
}

// Starts the helper and waits until the model is loaded (the first time it also downloads the model)
fn start_server() -> Result<(), String> {
    let python = python_bin();
    if !python.is_file() {
        return Err("The fast Parakeet engine (Python packages) is not installed. Run the installer again.".to_string());
    }
    fs::create_dir_all(data_dir()).map_err(|e| format!("Could not create the data folder: {}", e))?;
    let script = data_dir().join("parakeet_server.py");
    fs::write(&script, SERVER_SCRIPT).map_err(|e| format!("Could not write the helper: {}", e))?;
    let log = fs::File::create(super::config_dir().join("parakeet-server.log"))
        .map_err(|e| format!("Could not create the log file: {}", e))?;

    let mut cmd = Command::new(&python);
    cmd.arg("-u")
        .arg(&script)
        .env("HF_HOME", data_dir().join("hf"))
        .env("PYTHONUNBUFFERED", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(log));
    if onnx_marker().is_file() {
        cmd.env("HF_HUB_OFFLINE", "1"); // already downloaded: no network needed
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Could not start the Parakeet engine: {}", e))?;
    if let Some(st) = DOWNLOAD.lock().unwrap().as_mut() {
        if st.running {
            st.pid = child.id();
        }
    }
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Could not talk to the Parakeet engine.".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Could not read from the Parakeet engine.".to_string())?;
    let mut reader = BufReader::new(stdout);
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).unwrap_or(0);
        if n == 0 {
            let _ = child.kill();
            let _ = child.wait();
            return Err("The Parakeet engine did not start. See parakeet-server.log in the settings folder.".to_string());
        }
        if line.trim() == "READY" {
            break;
        }
    }
    *SERVER.lock().unwrap() = Some(Server {
        child,
        stdin,
        stdout: reader,
        last_used: Instant::now(),
    });
    let _ = fs::write(onnx_marker(), "ready");
    Ok(())
}

fn ensure_server() -> Result<(), String> {
    let began = Instant::now();
    loop {
        if SERVER.lock().unwrap().is_some() {
            return Ok(());
        }
        if !SERVER_STARTING.swap(true, Ordering::SeqCst) {
            let result = start_server();
            SERVER_STARTING.store(false, Ordering::SeqCst);
            return result;
        }
        if began.elapsed() > Duration::from_secs(900) {
            return Err("The Parakeet engine took too long to start.".to_string());
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn stop_server() {
    if let Some(mut s) = SERVER.lock().unwrap().take() {
        let _ = s.child.kill();
        let _ = s.child.wait();
    }
}

// Frees the memory (about 1.6 GB) when nothing was dictated for ten minutes
pub fn start_idle_watch() {
    thread::spawn(|| loop {
        thread::sleep(Duration::from_secs(30));
        let mut guard = SERVER.lock().unwrap();
        let idle = guard
            .as_ref()
            .map_or(false, |s| s.last_used.elapsed() > Duration::from_secs(600));
        if idle && !is_recording() {
            if let Some(mut s) = guard.take() {
                let _ = s.child.kill();
                let _ = s.child.wait();
            }
        }
    });
}

fn server_recognize(pcm: &[u8]) -> Result<String, String> {
    ensure_server()?;
    let mut guard = SERVER.lock().unwrap();
    let reply: Option<String> = match guard.as_mut() {
        Some(server) => {
            let len = (pcm.len() & !1usize) as u32;
            let outcome: std::io::Result<String> = (|| -> std::io::Result<String> {
                server.stdin.write_all(&len.to_le_bytes())?;
                server.stdin.write_all(&pcm[..len as usize])?;
                server.stdin.flush()?;
                let mut line = String::new();
                server.stdout.read_line(&mut line)?;
                Ok(line)
            })();
            match outcome {
                Ok(line) if !line.is_empty() => {
                    server.last_used = Instant::now();
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
            match t.strip_prefix("ERR ") {
                Some(msg) => Err(format!("The Parakeet engine reported: {}", msg)),
                None => Ok(t.to_string()),
            }
        }
        None => {
            if let Some(mut dead) = guard.take() {
                let _ = dead.child.kill();
                let _ = dead.child.wait();
            }
            Err("The Parakeet engine stopped. Try again.".to_string())
        }
    }
}

// ------------------------------------------------------------ sounds

fn tone(freq: f32, ms: u32, rate: u32, out: &mut Vec<i16>) {
    let n = (rate as u64 * ms as u64 / 1000) as usize;
    let fade = ((rate as usize) * 8 / 1000).max(1); // 8 ms fade: no clicks
    for i in 0..n {
        let t = i as f32 / rate as f32;
        let mut a = (2.0 * std::f32::consts::PI * freq * t).sin() * 0.35;
        if i < fade {
            a *= i as f32 / fade as f32;
        }
        if i + fade > n {
            a *= (n - i) as f32 / fade as f32;
        }
        out.push((a * 32767.0) as i16);
    }
}

fn sound_wav(parts: &[(f32, u32)]) -> Vec<u8> {
    let rate = 44100u32;
    let mut samples: Vec<i16> = Vec::new();
    for &(freq, ms) in parts {
        tone(freq, ms, rate, &mut samples);
    }
    let data_len = (samples.len() * 2) as u32;
    let mut v: Vec<u8> = Vec::new();
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data_len).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&rate.to_le_bytes());
    v.extend_from_slice(&(rate * 2).to_le_bytes());
    v.extend_from_slice(&2u16.to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        v.extend_from_slice(&s.to_le_bytes());
    }
    v
}

fn sound_player_available() -> bool {
    have("paplay") || have("pw-play") || have("pw-cat") || have("aplay")
}

// Plays a short beep and waits until it is finished. Ok(player used) or Err(why not).
fn play_report(name: &str) -> Result<String, String> {
    let parts: &[(f32, u32)] = match name {
        "start" => &[(660.0, 70), (880.0, 90)],
        "stop" => &[(880.0, 70), (660.0, 90)],
        _ => &[(300.0, 220)],
    };
    let path = super::config_dir().join(format!("sound-{}.wav", name));
    if !path.is_file() {
        fs::write(&path, sound_wav(parts)).map_err(|e| format!("Could not write the sound file: {}", e))?;
    }
    let file = path.to_string_lossy().to_string();
    let mut tried = false;
    let mut last_error = String::new();
    for (tool, args) in [
        ("paplay", Vec::<&str>::new()),
        ("pw-play", Vec::<&str>::new()),
        ("pw-cat", vec!["--playback"]),
        ("aplay", vec!["-q"]),
    ] {
        if !have(tool) {
            continue;
        }
        tried = true;
        let output = gui_cmd(tool)
            .args(&args)
            .arg(&file)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output();
        match output {
            Ok(o) if o.status.success() => return Ok(tool.to_string()),
            Ok(o) => {
                last_error = format!("{}: {}", tool, String::from_utf8_lossy(&o.stderr).trim());
            }
            Err(e) => {
                last_error = format!("{}: {}", tool, e);
            }
        }
    }
    if tried {
        Err(last_error)
    } else {
        Err("No sound player found (install pulseaudio-utils or alsa-utils).".to_string())
    }
}

// The "Play test sound" button
pub fn test_sound() -> Result<String, String> {
    play_report("start")
}

fn play_blocking(name: &str) -> bool {
    play_report(name).is_ok()
}

// If no sound can be played, the start of the listening is shown as a notification instead
fn beep(cfg: &VoiceConfig, name: &'static str, wait: bool) {
    if !cfg.sounds {
        return;
    }
    let work = move || {
        if !play_blocking(name) && name == "start" {
            notify("Listening...", "Speak now.");
        }
    };
    if wait {
        work();
    } else {
        thread::spawn(work);
    }
}

// ------------------------------------------------------------ recording

struct Recorder {
    child: Child,
    audio: Arc<Mutex<Vec<u8>>>,
    reader: Option<JoinHandle<()>>,
}

impl Recorder {
    // Stops recording and returns the audio (16 kHz, mono, 16-bit)
    fn finish(mut self) -> Vec<u8> {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(h) = self.reader.take() {
            let _ = h.join();
        }
        let bytes = self.audio.lock().unwrap().clone();
        bytes
    }
}

fn start_recorder() -> Result<Recorder, String> {
    let (program, args): (&str, Vec<&str>) = match recorder_tool() {
        Some("parec") => (
            "parec",
            vec!["--rate=16000", "--channels=1", "--format=s16le", "--latency-msec=50"],
        ),
        Some(_) => (
            "arecord",
            vec!["-q", "-t", "raw", "-f", "S16_LE", "-r", "16000", "-c", "1"],
        ),
        None => {
            return Err(
                "No recording tool found. Install pulseaudio-utils (parec) or alsa-utils (arecord)."
                    .to_string(),
            )
        }
    };
    let mut child = gui_cmd(program)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Could not start the microphone recording: {}", e))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Could not read the microphone.".to_string())?;
    let audio = Arc::new(Mutex::new(Vec::<u8>::new()));
    let sink = audio.clone();
    let reader = thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match stdout.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => sink.lock().unwrap().extend_from_slice(&buf[..n]),
            }
        }
    });
    Ok(Recorder {
        child,
        audio,
        reader: Some(reader),
    })
}

// ------------------------------------------------------------ splitting speech into phrases

const FRAME_BYTES: usize = 640; // 20 ms of 16 kHz, 16-bit, mono

// Cuts the audio into phrases at pauses, so every phrase is read exactly once
struct Splitter {
    carry: Vec<u8>,
    phrase: Vec<u8>,
    preroll: VecDeque<Vec<u8>>,
    in_phrase: bool,
    speech_frames: usize,
    silence_frames: usize,
    pause_frames: usize,
    noise: f32,
}

impl Splitter {
    fn new(pause_ms: u32) -> Splitter {
        Splitter {
            carry: Vec::new(),
            phrase: Vec::new(),
            preroll: VecDeque::new(),
            in_phrase: false,
            speech_frames: 0,
            silence_frames: 0,
            pause_frames: ((pause_ms / 20).max(5)) as usize,
            noise: 150.0,
        }
    }

    fn frame_level(frame: &[u8]) -> f32 {
        let mut sum = 0f64;
        let mut n = 0f64;
        for c in frame.chunks_exact(2) {
            let s = i16::from_le_bytes([c[0], c[1]]) as f64;
            sum += s * s;
            n += 1.0;
        }
        if n == 0.0 {
            0.0
        } else {
            (sum / n).sqrt() as f32
        }
    }

    // The phrase that is still being spoken (None if nothing that sounds like speech yet)
    fn current(&self) -> Option<&[u8]> {
        if self.in_phrase && self.speech_frames >= 6 {
            Some(&self.phrase)
        } else {
            None
        }
    }

    // True while the speaker has just gone quiet (the phrase may end any moment)
    fn in_pause(&self) -> bool {
        self.in_phrase && self.silence_frames >= 4
    }

    // Ends the current phrase; None if it was too short to be speech
    fn take_phrase(&mut self) -> Option<Vec<u8>> {
        let enough = self.speech_frames >= 6;
        let mut phrase = std::mem::take(&mut self.phrase);
        // keep only a short tail of the silence that ended the phrase
        let drop = self.silence_frames.saturating_sub(5) * FRAME_BYTES;
        if drop < phrase.len() {
            let keep = phrase.len() - drop;
            phrase.truncate(keep);
        }
        self.in_phrase = false;
        self.speech_frames = 0;
        self.silence_frames = 0;
        if enough {
            Some(phrase)
        } else {
            None
        }
    }

    fn feed(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        self.carry.extend_from_slice(bytes);
        let mut done: Vec<Vec<u8>> = Vec::new();
        while self.carry.len() >= FRAME_BYTES {
            let frame: Vec<u8> = self.carry.drain(..FRAME_BYTES).collect();
            let level = Self::frame_level(&frame);
            let threshold = (self.noise * 3.0).max(300.0);
            let speaking = level > threshold;
            if !speaking {
                // learn how loud the room is
                self.noise = self.noise * 0.98 + level * 0.02;
            }
            if speaking {
                if !self.in_phrase {
                    self.in_phrase = true;
                    self.phrase.clear();
                    for f in self.preroll.drain(..) {
                        self.phrase.extend_from_slice(&f);
                    }
                }
                self.phrase.extend_from_slice(&frame);
                self.speech_frames += 1;
                self.silence_frames = 0;
            } else if self.in_phrase {
                self.phrase.extend_from_slice(&frame);
                self.silence_frames += 1;
                if self.silence_frames >= self.pause_frames {
                    if let Some(p) = self.take_phrase() {
                        done.push(p);
                    }
                }
            } else {
                self.preroll.push_back(frame);
                if self.preroll.len() > 10 {
                    self.preroll.pop_front();
                }
            }
            // never let one phrase grow beyond about 25 seconds
            if self.in_phrase && self.phrase.len() > FRAME_BYTES * 50 * 25 {
                if let Some(p) = self.take_phrase() {
                    done.push(p);
                }
            }
        }
        done
    }

    // The recording stopped: whatever is left is the last phrase
    fn flush(&mut self) -> Option<Vec<u8>> {
        if self.in_phrase {
            self.take_phrase()
        } else {
            None
        }
    }
}

// ------------------------------------------------------------ speech recognition

fn peak(pcm: &[u8]) -> i32 {
    pcm.chunks_exact(2)
        .map(|c| (i16::from_le_bytes([c[0], c[1]]) as i32).abs())
        .max()
        .unwrap_or(0)
}

fn write_wav(path: &Path, pcm: &[u8]) -> std::io::Result<()> {
    let mut f = fs::File::create(path)?;
    let data_len = pcm.len() as u32;
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data_len).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?; // size of the fmt chunk
    f.write_all(&1u16.to_le_bytes())?; // PCM
    f.write_all(&1u16.to_le_bytes())?; // mono
    f.write_all(&16000u32.to_le_bytes())?; // sample rate
    f.write_all(&32000u32.to_le_bytes())?; // bytes per second
    f.write_all(&2u16.to_le_bytes())?; // bytes per sample frame
    f.write_all(&16u16.to_le_bytes())?; // bits per sample
    f.write_all(b"data")?;
    f.write_all(&data_len.to_le_bytes())?;
    f.write_all(pcm)?;
    Ok(())
}

fn thread_count() -> usize {
    thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(8)
}

// "[00:00:00.000 --> 00:00:03.000]  text" -> "text"
fn strip_timestamp(line: &str) -> &str {
    let t = line.trim();
    if t.starts_with('[') {
        if let Some(i) = t.find(']') {
            if t[..i].contains("-->") {
                return t[i + 1..].trim();
            }
        }
    }
    t
}

fn clean_transcript(raw: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    for line in raw.lines() {
        let t = strip_timestamp(line);
        if t.is_empty() {
            continue;
        }
        // markers such as [BLANK_AUDIO] or (music), and engine log lines
        if (t.starts_with('[') && t.ends_with(']')) || (t.starts_with('(') && t.ends_with(')')) {
            continue;
        }
        if t.starts_with("ggml_")
            || t.starts_with("whisper_")
            || t.starts_with("parakeet_")
            || t.starts_with("system_info")
            || t.starts_with("main:")
        {
            continue;
        }
        parts.push(t.to_string());
    }
    parts.join(" ")
}

// Characters the virtual keyboard can type
fn for_typing(s: &str) -> String {
    let mapped: String = s
        .chars()
        .map(|c| match c {
            '\u{2018}' | '\u{2019}' => '\'',
            '\u{201c}' | '\u{201d}' | '\u{201e}' => '"',
            '\u{2013}' | '\u{2014}' => '-',
            '\u{a0}' => ' ',
            _ => c,
        })
        .collect();
    mapped.replace('\u{2026}', "...")
}

// Reads the speech in `pcm` with the selected model
fn run_engine(pcm: &[u8], cfg: &VoiceConfig, tag: &str) -> Result<String, String> {
    if pcm.len() < 8000 {
        return Ok(String::new()); // shorter than a quarter of a second
    }
    if peak(pcm) < 300 {
        return Ok(String::new()); // silence
    }
    let engine = model_engine(&cfg.model).ok_or_else(|| "Unknown model.".to_string())?;
    if engine == Engine::ParakeetOnnx {
        if !model_ready(&cfg.model) {
            return Err("The fast Parakeet engine is not set up yet. Open the Voice tab and press Download on it.".to_string());
        }
        return server_recognize(pcm).map(|t| clean_transcript(&t));
    }
    let bin = engine_binary(engine).ok_or_else(|| missing_engine_message(engine))?;
    let model = model_file(&cfg.model)
        .filter(|p| p.is_file())
        .ok_or_else(|| "The selected model is not downloaded.".to_string())?;
    let wav = std::env::temp_dir().join(format!(
        "expander-voice-{}-{}.wav",
        std::process::id(),
        tag
    ));
    write_wav(&wav, pcm).map_err(|e| format!("Could not write the audio: {}", e))?;

    let mut cmd = Command::new(&bin);
    cmd.arg("-m").arg(&model).arg("-f").arg(&wav);
    if engine == Engine::Whisper {
        cmd.arg("-l")
            .arg(&cfg.language)
            .arg("-t")
            .arg(thread_count().to_string())
            .arg("-nt")
            .arg("-np");
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = cmd.output();
    let _ = fs::remove_file(&wav);
    let output = output.map_err(|e| format!("Could not run the voice engine: {}", e))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr).to_string();
        let last = err
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim()
            .to_string();
        return Err(format!("The voice engine failed: {}", last));
    }
    Ok(clean_transcript(&String::from_utf8_lossy(&output.stdout)))
}

// ------------------------------------------------------------ timing information

fn stats_path() -> PathBuf {
    super::config_dir().join("voice_stats.txt")
}

// The last few readings of the background service (shown in the Voice tab)
pub fn read_stats() -> String {
    fs::read_to_string(stats_path()).unwrap_or_default()
}

fn log_stat(line: String) {
    let mut lines: Vec<String> = fs::read_to_string(stats_path())
        .unwrap_or_default()
        .lines()
        .map(|l| l.to_string())
        .collect();
    lines.push(line);
    while lines.len() > 8 {
        lines.remove(0);
    }
    let _ = fs::write(stats_path(), lines.join("\n"));
}

fn record_stats(_lines: &mut Vec<String>, kind: &str, audio_bytes: usize, secs: f32) {
    log_stat(format!(
        "{}: {:.1} s to read {:.1} s of speech",
        kind,
        secs,
        audio_bytes as f32 / 32000.0
    ));
}

// Hands text to the typing keyboard and notes it in the log
fn type_and_log(text: &str) {
    let typed = for_typing(text);
    let count = typed.chars().count();
    let sent = super::input::type_text(typed);
    if sent {
        log_stat(format!("typed {} characters", count));
    } else {
        log_stat(format!("could not type {} characters: the typing keyboard is not ready", count));
    }
}

static LAST_WARM: Mutex<Option<Instant>> = Mutex::new(None);

// Reads the model file once so that the first phrase does not wait for the disk
fn warm_model(id: &str) {
    let path = match model_file(id) {
        Some(p) => p,
        None => return,
    };
    {
        let mut last = LAST_WARM.lock().unwrap();
        if (*last).map_or(false, |t| t.elapsed() < Duration::from_secs(300)) {
            return;
        }
        *last = Some(Instant::now());
    }
    thread::spawn(move || {
        if let Ok(mut f) = fs::File::open(&path) {
            let _ = std::io::copy(&mut f, &mut std::io::sink());
        }
    });
}

// ------------------------------------------------------------ live text

fn last_chars(text: &str, n: usize) -> String {
    let count = text.chars().count();
    if count <= n {
        text.to_string()
    } else {
        let tail: String = text.chars().skip(count - n).collect();
        format!("...{}", tail)
    }
}

fn notify_live(title: &str, body: &str, id: &mut Option<String>) {
    let mut cmd = gui_cmd("notify-send");
    cmd.args(["-a", "Text Expander", "-t", "20000", "-p"]);
    if let Some(i) = id.as_ref() {
        cmd.arg("-r").arg(i);
    }
    cmd.arg(title)
        .arg(body)
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    if let Ok(out) = cmd.output() {
        if id.is_none() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                *id = Some(s);
            }
        }
    }
}

fn show_live(style: &str, text: &str, id: &mut Option<String>) {
    let tail = last_chars(text, 200);
    if style == "box" {
        let _ = fs::write(live_path(), tail);
    } else {
        notify_live("Listening...", &tail, id);
    }
}

// The box on screen is a small separate window (the program started with --overlay).
// It is forced onto X11 (Xwayland) so that it can stay on top without taking the keyboard focus.
fn spawn_overlay() -> Option<Child> {
    let exe = std::env::current_exe().ok()?;
    let exe_str = exe.to_str()?.to_string();
    let _ = fs::write(live_path(), "");
    let _ = fs::remove_file(super::config_dir().join("voice_close"));
    let _ = fs::remove_file(super::config_dir().join("voice_note.txt"));
    let _ = fs::write(super::config_dir().join("voice_level.txt"), "0");
    let mut cmd = gui_cmd(&exe_str);
    cmd.arg("--overlay")
        .env_remove("WAYLAND_DISPLAY")
        .env("WINIT_UNIX_BACKEND", "x11")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = cmd.spawn().ok()?;
    thread::sleep(Duration::from_millis(700));
    match child.try_wait() {
        Ok(None) => Some(child),
        _ => None,
    }
}

static REVIEW_OPEN: AtomicBool = AtomicBool::new(false);

// Shows the text in a window where it can be corrected. What is sent from there is typed at the cursor.
fn review_and_type(text: String) {
    if REVIEW_OPEN.swap(true, Ordering::SeqCst) {
        return;
    }
    let input = super::config_dir().join("voice_review_in.txt");
    let output = super::config_dir().join("voice_review_out.txt");
    let _ = fs::write(&input, &text);
    let _ = fs::remove_file(&output);
    let exe = std::env::current_exe()
        .ok()
        .and_then(|e| e.to_str().map(|s| s.to_string()));
    let mut opened = false;
    if let Some(exe_str) = exe {
        let spawned = gui_cmd(&exe_str)
            .arg("--review")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        if let Ok(mut child) = spawned {
            opened = true;
            let _ = child.wait();
        }
    }
    let sent = fs::read_to_string(&output).ok();
    let _ = fs::remove_file(&output);
    let _ = fs::remove_file(&input);
    REVIEW_OPEN.store(false, Ordering::SeqCst);
    if !opened {
        notify("Voice typing", "Could not open the edit box, so the text is typed directly.");
        type_and_log(&text);
        return;
    }
    if let Some(final_text) = sent {
        if !final_text.trim().is_empty() {
            // give the focus time to go back to the program the text is meant for
            thread::sleep(Duration::from_millis(350));
            type_and_log(final_text.trim());
        }
    }
}

// Asks the box to close itself (a clean close, so the desktop removes it at once)
fn close_overlay(mut child: Child) {
    let marker = super::config_dir().join("voice_close");
    let _ = fs::write(&marker, "close");
    for _ in 0..40 {
        if let Ok(Some(_)) = child.try_wait() {
            let _ = fs::remove_file(&marker);
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
    let _ = fs::remove_file(&marker);
}

fn note_path() -> PathBuf {
    super::config_dir().join("voice_note.txt")
}

fn level_path() -> PathBuf {
    super::config_dir().join("voice_level.txt")
}

// ------------------------------------------------------------ what fits this computer

pub struct Hardware {
    pub cpu: String,
    pub cores: usize,
    pub ram_mb: u64,
}

pub fn hardware() -> Hardware {
    let cpu = fs::read_to_string("/proc/cpuinfo")
        .unwrap_or_default()
        .lines()
        .find(|l| l.starts_with("model name"))
        .and_then(|l| l.split(':').nth(1))
        .map(|v| v.trim().to_string())
        .unwrap_or_else(|| "unknown processor".to_string());
    let ram_kb: u64 = fs::read_to_string("/proc/meminfo")
        .unwrap_or_default()
        .lines()
        .find(|l| l.starts_with("MemTotal:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let cores = thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    Hardware {
        cpu,
        cores,
        ram_mb: ram_kb / 1024,
    }
}

// A first guess from the hardware alone
pub fn suggested_model(hw: &Hardware) -> &'static str {
    if hw.ram_mb >= 4000 && hw.cores >= 2 {
        "parakeet-onnx"
    } else if hw.ram_mb < 3500 || hw.cores <= 2 {
        "parakeet-q4_0"
    } else if hw.ram_mb < 6500 || hw.cores <= 4 {
        "parakeet-q4_k"
    } else {
        "parakeet-q8_0"
    }
}

// Records 5 seconds, reads the same audio with every downloaded model and times each one.
// Returns the text for the window and the best model that is fast enough.
pub fn benchmark(
    cfg: VoiceConfig,
    progress: Arc<Mutex<String>>,
) -> Result<(String, Option<String>), String> {
    if is_recording() {
        return Err("A recording is already running.".to_string());
    }
    let set = |m: &str| {
        *progress.lock().unwrap() = m.to_string();
    };
    set("Speak for 5 seconds...");
    beep(&cfg, "start", true);
    let recorder = start_recorder()?;
    thread::sleep(Duration::from_secs(5));
    let pcm = recorder.finish();
    beep(&cfg, "stop", true);
    if peak(&pcm) < 300 {
        return Err("I could not hear you. Speak clearly for the 5 seconds and try again.".to_string());
    }
    let audio_secs = pcm.len() as f32 / 32000.0;

    let mut rows: Vec<(String, f32)> = Vec::new();
    for m in MODELS {
        if !model_ready(m.id) {
            continue;
        }
        set(&format!("Reading with {} ...", m.id));
        let mut c = cfg.clone();
        c.model = m.id.to_string();
        if m.engine == Engine::ParakeetOnnx {
            // do not count the one-time start of the helper
            let _ = ensure_server();
            let _ = run_engine(&pcm, &c, "bench");
        }
        let started = Instant::now();
        if run_engine(&pcm, &c, "bench").is_ok() {
            rows.push((m.id.to_string(), started.elapsed().as_secs_f32()));
        }
    }
    if rows.is_empty() {
        return Err("No downloaded model could be tested. Download a model first.".to_string());
    }

    // From best quality to worst; the best one that is fast enough wins
    let order = [
        "large-v3",
        "large-v3-q5_0",
        "large-v3-turbo",
        "parakeet-onnx",
        "parakeet-q8_0",
        "large-v3-turbo-q5_0",
        "parakeet-q4_k",
        "medium-q5_0",
        "parakeet-q4_0",
        "small-q5_1",
        "base-q5_1",
        "tiny-q5_1",
    ];
    let fast_enough = audio_secs * 0.4;
    let mut best: Option<String> = None;
    for id in order {
        if let Some((_, secs)) = rows.iter().find(|(r, _)| r == id) {
            if *secs <= fast_enough {
                best = Some(id.to_string());
                break;
            }
        }
    }
    if best.is_none() {
        best = rows
            .iter()
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|r| r.0.clone());
    }

    let mut text = format!("Tested with {:.1} s of your speech:\n", audio_secs);
    for (id, secs) in &rows {
        let verdict = if *secs <= fast_enough {
            "fast enough"
        } else if *secs <= audio_secs {
            "usable, with a delay"
        } else {
            "too slow"
        };
        text.push_str(&format!("  {}: {:.1} s - {}\n", id, secs, verdict));
    }
    Ok((text, best))
}

// ------------------------------------------------------------ one dictation

// Runs while recording: cuts the speech into phrases at every pause and reads each phrase once.
// Between pauses it also reads the phrase that is still being spoken, for the live text.
// Returns the phrases when the recording has stopped.
fn spawn_worker(
    audio: Arc<Mutex<Vec<u8>>>,
    stop: Arc<AtomicBool>,
    had_error: Arc<AtomicBool>,
    cfg: VoiceConfig,
    style: String,
    type_live: bool,
) -> JoinHandle<Vec<String>> {
    thread::spawn(move || {
        let mut splitter = Splitter::new(cfg.pause_ms);
        let mut cursor = 0usize;
        let mut phrases: Vec<String> = Vec::new();
        let mut notif_id: Option<String> = None;
        let mut stats: Vec<String> = Vec::new();
        let mut last_preview_len = 0usize;
        let mut preview_on = !style.is_empty();
        let mut slow_previews = 0u32;
        let mut last_level_write = Instant::now();
        loop {
            let stopping = stop.load(Ordering::SeqCst);
            let fresh: Vec<u8> = {
                let a = audio.lock().unwrap();
                let end = a.len() & !1usize;
                if end > cursor {
                    let v = a[cursor..end].to_vec();
                    cursor = end;
                    v
                } else {
                    Vec::new()
                }
            };
            // how loud it is right now (for the level bar in the box)
            if style == "box" && !fresh.is_empty() && last_level_write.elapsed() > Duration::from_millis(100) {
                last_level_write = Instant::now();
                let tail = &fresh[fresh.len().saturating_sub(3200)..];
                let level = (Splitter::frame_level(tail) / 3000.0).min(1.0);
                let _ = fs::write(level_path(), format!("{:.2}", level));
            }
            let mut ready = splitter.feed(&fresh);
            if stopping {
                if let Some(last) = splitter.flush() {
                    ready.push(last);
                }
            }
            if !ready.is_empty() {
                last_preview_len = 0;
            }
            for pcm in ready {
                let started = Instant::now();
                match run_engine(&pcm, &cfg, "phrase") {
                    Ok(text) => {
                        record_stats(&mut stats, "phrase", pcm.len(), started.elapsed().as_secs_f32());
                        if !text.is_empty() {
                            phrases.push(text.clone());
                            if !style.is_empty() {
                                show_live(&style, &phrases.join(" "), &mut notif_id);
                            }
                            if type_live {
                                type_and_log(&text);
                            }
                        }
                    }
                    Err(e) => {
                        had_error.store(true, Ordering::SeqCst);
                        beep(&cfg, "error", false);
                        notify("Voice typing failed", &e);
                    }
                }
            }

            // Live preview of the phrase that is still being spoken. It is skipped when the
            // speaker has just paused (the phrase is about to be finished anyway), and it
            // switches itself off for the rest of the recording if reading is too slow.
            if !stopping && preview_on && !splitter.in_pause() {
                let current: Option<Vec<u8>> = splitter.current().map(|c| c.to_vec());
                if let Some(cur) = current {
                    if cur.len() >= last_preview_len + 24000 {
                        last_preview_len = cur.len();
                        let started = Instant::now();
                        if let Ok(text) = run_engine(&cur, &cfg, "preview") {
                            let took = started.elapsed().as_secs_f32();
                            record_stats(&mut stats, "preview", cur.len(), took);
                            if !text.is_empty() && !stop.load(Ordering::SeqCst) {
                                let mut shown = phrases.join(" ");
                                if !shown.is_empty() {
                                    shown.push(' ');
                                }
                                shown.push_str(&text);
                                show_live(&style, &shown, &mut notif_id);
                            }
                            if took > 2.5 {
                                slow_previews += 1;
                                if slow_previews >= 2 {
                                    preview_on = false;
                                    let _ = fs::write(
                                        note_path(),
                                        "Words while you speak are paused: this computer reads slowly. Pick a faster model in the Voice tab.",
                                    );
                                }
                            }
                        }
                    }
                }
            }
            if stopping {
                break;
            }
            thread::sleep(Duration::from_millis(40));
        }
        phrases
    })
}

struct Session {
    recorder: Recorder,
    stop: Arc<AtomicBool>,
    had_error: Arc<AtomicBool>,
    worker: JoinHandle<Vec<String>>,
    overlay: Option<Child>,
    type_live: bool,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);

pub fn is_recording() -> bool {
    SESSION.lock().unwrap().is_some()
}

pub fn start(cfg: &VoiceConfig) {
    let mut guard = SESSION.lock().unwrap();
    if guard.is_some() {
        return;
    }
    if REVIEW_OPEN.load(Ordering::SeqCst) {
        notify("Voice typing", "Send or close the edit box first.");
        return;
    }
    if !model_ready(&cfg.model) {
        beep(cfg, "error", false);
        notify(
            "Voice typing",
            "Download a model first: open Text Expander > Voice typing.",
        );
        return;
    }
    let engine = model_engine(&cfg.model).unwrap_or(Engine::Parakeet);
    if !engine_available(engine) {
        beep(cfg, "error", false);
        notify("Voice typing", &missing_engine_message(engine));
        return;
    }
    if engine == Engine::ParakeetOnnx {
        // load the helper while the first words are spoken
        thread::spawn(|| {
            let _ = ensure_server();
        });
    } else {
        warm_model(&cfg.model);
    }
    // The start beep plays first, so that the microphone does not pick it up
    beep(cfg, "start", true);
    let recorder = match start_recorder() {
        Ok(r) => r,
        Err(e) => {
            beep(cfg, "error", false);
            notify("Voice typing", &e);
            return;
        }
    };
    let stop = Arc::new(AtomicBool::new(false));
    let had_error = Arc::new(AtomicBool::new(false));
    let mut overlay: Option<Child> = None;
    let mut style = String::new();
    if cfg.live {
        style = cfg.live_style.clone();
        if style == "box" {
            overlay = spawn_overlay();
            if overlay.is_none() {
                style = "notification".to_string(); // the box could not start
            }
        }
    } else if !cfg.sounds {
        notify("Listening...", "Speak now.");
    }
    // Typing while a key is held would turn the text into shortcuts, so only in toggle mode.
    // With the edit box nothing is typed before the user sends the text.
    let type_live = cfg.type_live && cfg.mode == "toggle" && !cfg.review;
    let worker = spawn_worker(
        recorder.audio.clone(),
        stop.clone(),
        had_error.clone(),
        cfg.clone(),
        style,
        type_live,
    );
    *guard = Some(Session {
        recorder,
        stop,
        had_error,
        worker,
        overlay,
        type_live,
    });
}

// Stop recording; the last phrase is read and typed
pub fn stop_and_type(cfg: VoiceConfig) {
    let session = SESSION.lock().unwrap().take();
    let session = match session {
        Some(s) => s,
        None => return,
    };
    let Session {
        recorder,
        stop,
        had_error,
        worker,
        overlay,
        type_live,
    } = session;
    beep(&cfg, "stop", false);
    let _ = recorder.finish(); // the worker reads the rest of the audio
    stop.store(true, Ordering::SeqCst);
    let review = cfg.review;
    // close the box right away; the last phrase is still being read
    if let Some(child) = overlay {
        thread::spawn(move || close_overlay(child));
    }
    thread::spawn(move || {
        let phrases = worker.join().unwrap_or_default();
        let _ = fs::remove_file(live_path());
        if phrases.is_empty() {
            if !had_error.load(Ordering::SeqCst) {
                notify("Voice typing", "No speech detected.");
            }
            return;
        }
        if review {
            review_and_type(phrases.join(" "));
        } else if !type_live {
            thread::sleep(Duration::from_millis(150));
            type_and_log(&phrases.join(" "));
        }
    });
}

pub fn toggle(cfg: VoiceConfig) {
    if is_recording() {
        stop_and_type(cfg);
    } else {
        start(&cfg);
    }
}

// Used when a held key is released
pub fn stop_if_recording(cfg: VoiceConfig) {
    if is_recording() {
        stop_and_type(cfg);
    }
}

// Used by the "Test" button in the window: records, shows the text and how long it took, types nothing
pub fn test_once(cfg: VoiceConfig, seconds: u64) -> Result<String, String> {
    if is_recording() {
        return Err("A recording is already running.".to_string());
    }
    beep(&cfg, "start", true);
    let recorder = start_recorder()?;
    thread::sleep(Duration::from_secs(seconds));
    let pcm = recorder.finish();
    beep(&cfg, "stop", true);
    let started = Instant::now();
    let text = run_engine(&pcm, &cfg, "test")?;
    if text.is_empty() {
        Ok(text)
    } else {
        Ok(format!("{}  ({:.1} s)", text, started.elapsed().as_secs_f32()))
    }
}
