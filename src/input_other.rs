use super::Shared;
use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use rdev::{listen, Event, EventType, Key as RKey};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Sender},
        Arc,
    },
    thread,
    time::Duration,
};

// (characters to erase, replacement text, add a trailing space)
type Job = (usize, String, bool);

// Typing worker: erases the trigger and types the replacement
fn spawn_typer(busy: Arc<AtomicBool>) -> Sender<Job> {
    let (tx, rx) = mpsc::channel::<Job>();
    thread::spawn(move || {
        // On macOS this fails until Accessibility permission is granted: keep retrying
        let mut enigo = loop {
            match Enigo::new(&Settings::default()) {
                Ok(e) => break e,
                Err(e) => {
                    eprintln!("Typing backend not ready ({:?}); retrying in 3 seconds", e);
                    thread::sleep(Duration::from_secs(3));
                }
            }
        };
        for (erase_count, text, trailing_space) in rx {
            busy.store(true, Ordering::SeqCst);
            thread::sleep(Duration::from_millis(30));
            for _ in 0..erase_count {
                let _ = enigo.key(Key::Backspace, Direction::Click);
            }
            let out = if trailing_space { format!("{} ", text) } else { text };
            let _ = enigo.text(&out);
            thread::sleep(Duration::from_millis(150));
            busy.store(false, Ordering::SeqCst);
        }
    });
    tx
}

// The callback that sees every key press
fn make_callback(
    entries: Shared,
    enabled: Arc<AtomicBool>,
    busy: Arc<AtomicBool>,
    tx: Sender<Job>,
) -> impl FnMut(Event) + Send + 'static {
    let mut buffer = String::new();
    let mut cmd_ctrl = 0i32; // macOS: how many of Command/Control are held
    let track = cfg!(target_os = "macos");
    move |event: Event| {
        // On macOS, Command/Control + key is a shortcut, not typing.
        // (On Windows, AltGr is reported as Ctrl+Alt, so this is not used there.)
        match event.event_type {
            EventType::KeyPress(RKey::MetaLeft)
            | EventType::KeyPress(RKey::MetaRight)
            | EventType::KeyPress(RKey::ControlLeft)
            | EventType::KeyPress(RKey::ControlRight)
                if track =>
            {
                cmd_ctrl += 1
            }
            EventType::KeyRelease(RKey::MetaLeft)
            | EventType::KeyRelease(RKey::MetaRight)
            | EventType::KeyRelease(RKey::ControlLeft)
            | EventType::KeyRelease(RKey::ControlRight)
                if track =>
            {
                cmd_ctrl = (cmd_ctrl - 1).max(0)
            }
            _ => {}
        }

        if busy.load(Ordering::SeqCst) || !enabled.load(Ordering::SeqCst) {
            return;
        }
        if let EventType::KeyPress(key) = event.event_type {
            if cmd_ctrl > 0 {
                buffer.clear();
                return;
            }
            match key {
                RKey::Space => {
                    let word = buffer.to_lowercase();
                    let found = entries
                        .lock()
                        .unwrap()
                        .iter()
                        .find(|e| e.space && e.trigger.to_lowercase() == word)
                        .map(|e| e.text.clone());
                    if let Some(text) = found {
                        let _ = tx.send((word.chars().count() + 1, text, true));
                    }
                    buffer.clear();
                }
                RKey::Backspace => {
                    buffer.pop();
                }
                RKey::Return | RKey::Tab | RKey::Escape => buffer.clear(),
                // Modifier keys on their own don't change the typed word.
                // (AltGr is reported as Ctrl+Alt on Windows, so Ctrl must not clear it.)
                RKey::ShiftLeft
                | RKey::ShiftRight
                | RKey::CapsLock
                | RKey::Alt
                | RKey::AltGr
                | RKey::ControlLeft
                | RKey::ControlRight
                | RKey::MetaLeft
                | RKey::MetaRight => {}
                _ => match event.name {
                    Some(s) if !s.is_empty() && s.chars().all(|c| !c.is_control()) => {
                        buffer.push_str(&s);
                        if buffer.chars().count() > 32 {
                            buffer.clear();
                        } else {
                            // Instant triggers (no Space needed)
                            let word = buffer.to_lowercase();
                            let found = entries
                                .lock()
                                .unwrap()
                                .iter()
                                .find(|e| !e.space && e.trigger.to_lowercase() == word)
                                .map(|e| e.text.clone());
                            if let Some(text) = found {
                                let _ = tx.send((word.chars().count(), text, false));
                                buffer.clear();
                            }
                        }
                    }
                    // Arrow keys, Delete, shortcuts, etc.: the word is interrupted
                    _ => buffer.clear(),
                },
            }
        }
    }
}

// Windows: the hook gets its own thread
#[cfg(windows)]
pub fn start_listener(
    entries: Shared,
    enabled: Arc<AtomicBool>,
    _layout: super::LayoutSetting,
    _ocr: super::OcrShared,
    _voice: super::VoiceShared,
    _speak: super::SpeakShared,
) {
    let busy = Arc::new(AtomicBool::new(false));
    let tx = spawn_typer(busy.clone());
    let callback = make_callback(entries, enabled, busy, tx);
    thread::spawn(move || {
        if let Err(e) = listen(callback) {
            eprintln!("Listener error: {:?}", e);
        }
    });
}

// macOS: must run on the main thread. Retries until Input Monitoring is allowed.
#[cfg(target_os = "macos")]
pub fn listen_blocking(
    entries: Shared,
    enabled: Arc<AtomicBool>,
    _layout: super::LayoutSetting,
    _ocr: super::OcrShared,
    _voice: super::VoiceShared,
    _speak: super::SpeakShared,
) {
    let busy = Arc::new(AtomicBool::new(false));
    let tx = spawn_typer(busy.clone());
    loop {
        let callback = make_callback(entries.clone(), enabled.clone(), busy.clone(), tx.clone());
        if let Err(e) = listen(callback) {
            eprintln!(
                "Key listener could not start ({:?}). Allow Input Monitoring for Text Expander in System Settings > Privacy & Security.",
                e
            );
        }
        thread::sleep(Duration::from_secs(3));
    }
}
