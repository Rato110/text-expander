use super::{OcrShared, SpeakShared, VoiceShared};
use std::{
    io::{Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    thread,
};

pub fn socket_path() -> PathBuf {
    dirs::runtime_dir()
        .unwrap_or_else(super::config_dir)
        .join("expander.sock")
}

// The background service listens for commands such as "voice-toggle"
pub fn start(ocr: OcrShared, voice: VoiceShared, speak: SpeakShared) {
    let path = socket_path();
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Could not open the command socket: {}", e);
            return;
        }
    };
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = match stream {
                Ok(s) => s,
                Err(_) => continue,
            };
            let mut text = String::new();
            let _ = stream.read_to_string(&mut text);
            let command = text.trim().to_string();
            let v = voice.lock().unwrap().clone();
            let o = ocr.lock().unwrap().clone();
            let sp = speak.lock().unwrap().clone();
            match command.as_str() {
                "voice-toggle" => {
                    thread::spawn(move || super::voice::toggle(v));
                }
                "voice-start" => {
                    thread::spawn(move || super::voice::start(&v));
                }
                "voice-stop" => {
                    thread::spawn(move || super::voice::stop_if_recording(v));
                }
                "ocr" => {
                    thread::spawn(move || super::ocr::run_with_notification(o));
                }
                "read-da" | "read-en" | "read-stop" | "read-pause" => {
                    let action = command.clone();
                    thread::spawn(move || super::speak::run_action(&action, sp));
                }
                _ => {}
            }
            let _ = stream.write_all(b"ok\n");
        }
    });
}

// expander --voice-toggle  (and --voice-start, --voice-stop, --ocr) sends the command here
pub fn send(command: &str) -> bool {
    match UnixStream::connect(socket_path()) {
        Ok(mut s) => {
            let _ = s.write_all(command.as_bytes());
            let _ = s.shutdown(std::net::Shutdown::Write);
            let mut reply = String::new();
            let _ = s.read_to_string(&mut reply);
            true
        }
        Err(_) => false,
    }
}
