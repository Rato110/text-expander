use super::Shared;
use evdev::{
    uinput::{VirtualDevice, VirtualDeviceBuilder},
    AttributeSet, EventType, InputEvent, Key,
};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::Duration,
};

const VNAME: &str = "expander-virtual-keyboard";
const SPACE: u16 = 57;
const BACKSPACE: u16 = 14;
const LSHIFT: u16 = 42;
const ALTGR: u16 = 100;

// (character, key code, needs Shift, needs AltGr)
type Sym = (char, u16, bool, bool);

// Letters a-z and digits 0-9: same key positions on US and Finnish layouts.
const CHARS: &[(u16, char)] = &[
    (30, 'a'), (48, 'b'), (46, 'c'), (32, 'd'), (18, 'e'), (33, 'f'), (34, 'g'),
    (35, 'h'), (23, 'i'), (36, 'j'), (37, 'k'), (38, 'l'), (50, 'm'), (49, 'n'),
    (24, 'o'), (25, 'p'), (16, 'q'), (19, 'r'), (31, 's'), (20, 't'), (22, 'u'),
    (47, 'v'), (17, 'w'), (45, 'x'), (21, 'y'), (44, 'z'),
    (2, '1'), (3, '2'), (4, '3'), (5, '4'), (6, '5'), (7, '6'), (8, '7'),
    (9, '8'), (10, '9'), (11, '0'),
];

const US: &[Sym] = &[
    ('!', 2, true, false), ('@', 3, true, false), ('#', 4, true, false),
    ('$', 5, true, false), ('%', 6, true, false), ('^', 7, true, false),
    ('&', 8, true, false), ('*', 9, true, false), ('(', 10, true, false),
    (')', 11, true, false),
    ('-', 12, false, false), ('_', 12, true, false),
    ('=', 13, false, false), ('+', 13, true, false),
    ('[', 26, false, false), ('{', 26, true, false),
    (']', 27, false, false), ('}', 27, true, false),
    (';', 39, false, false), (':', 39, true, false),
    ('\'', 40, false, false), ('"', 40, true, false),
    ('`', 41, false, false), ('~', 41, true, false),
    ('\\', 43, false, false), ('|', 43, true, false),
    (',', 51, false, false), ('<', 51, true, false),
    ('.', 52, false, false), ('>', 52, true, false),
    ('/', 53, false, false), ('?', 53, true, false),
];

// Finnish / Swedish (Nordic) layout
const FI: &[Sym] = &[
    ('!', 2, true, false), ('"', 3, true, false), ('@', 3, false, true),
    ('#', 4, true, false), ('$', 5, false, true), ('%', 6, true, false),
    ('&', 7, true, false), ('/', 8, true, false), ('{', 8, false, true),
    ('(', 9, true, false), ('[', 9, false, true), (')', 10, true, false),
    (']', 10, false, true), ('=', 11, true, false), ('}', 11, false, true),
    ('+', 12, false, false), ('?', 12, true, false), ('\\', 12, false, true),
    ('å', 26, false, false), ('ö', 39, false, false), ('ä', 40, false, false),
    ('\'', 43, false, false), ('*', 43, true, false),
    (',', 51, false, false), (';', 51, true, false),
    ('.', 52, false, false), (':', 52, true, false),
    ('-', 53, false, false), ('_', 53, true, false),
    ('<', 86, false, false), ('>', 86, true, false), ('|', 86, false, true),
];

// Danish layout
const DK: &[Sym] = &[
    ('!', 2, true, false), ('"', 3, true, false), ('@', 3, false, true),
    ('#', 4, true, false), ('$', 5, false, true), ('%', 6, true, false),
    ('&', 7, true, false), ('/', 8, true, false), ('{', 8, false, true),
    ('(', 9, true, false), ('[', 9, false, true), (')', 10, true, false),
    (']', 10, false, true), ('=', 11, true, false), ('}', 11, false, true),
    ('+', 12, false, false), ('?', 12, true, false),
    ('|', 13, false, true),
    ('å', 26, false, false), ('ø', 39, false, false), ('æ', 40, false, false),
    ('\'', 43, false, false), ('*', 43, true, false),
    (',', 51, false, false), (';', 51, true, false),
    ('.', 52, false, false), (':', 52, true, false),
    ('-', 53, false, false), ('_', 53, true, false),
    ('<', 86, false, false), ('>', 86, true, false), ('\\', 86, false, true),
];

#[derive(Clone, Copy)]
struct Keymap {
    name: &'static str,
    symbols: &'static [Sym],
}

impl Keymap {
    // What character does this key produce with the given modifiers?
    fn char_of(&self, code: u16, shift: bool, altgr: bool) -> Option<char> {
        if let Some((c, _, _, _)) = self
            .symbols
            .iter()
            .copied()
            .find(|&(_, k, s, a)| k == code && s == shift && a == altgr)
        {
            return Some(c);
        }
        if altgr {
            return None;
        }
        if let Some((_, c)) = CHARS.iter().copied().find(|&(k, _)| k == code) {
            if c.is_ascii_alphabetic() || !shift {
                return Some(c);
            }
            return None;
        }
        if shift {
            // Shifted ä/ö/å -> treat as the lowercase letter
            if let Some((c, _, _, _)) = self
                .symbols
                .iter()
                .copied()
                .find(|&(_, k, s, a)| k == code && !s && !a)
            {
                if c.is_alphabetic() {
                    return Some(c);
                }
            }
        }
        None
    }

    // Which key (and modifiers) types this character?
    fn keys_of(&self, c: char) -> Option<(u16, bool, bool)> {
        if c == ' ' {
            return Some((SPACE, false, false));
        }
        if let Some((_, k, s, a)) = self.symbols.iter().copied().find(|&(ch, _, _, _)| ch == c) {
            return Some((k, s, a));
        }
        let lower = c.to_lowercase().next()?;
        let shift = c != lower;
        if let Some((k, _)) = CHARS.iter().copied().find(|&(_, ch)| ch == lower) {
            return Some((k, shift, false));
        }
        self.symbols
            .iter()
            .copied()
            .find(|&(ch, _, s, a)| ch == lower && !s && !a)
            .map(|(_, k, _, _)| (k, shift, false))
    }
}

// English (UK) layout
const UK: &[Sym] = &[
    ('!', 2, true, false), ('"', 3, true, false), ('£', 4, true, false),
    ('$', 5, true, false), ('%', 6, true, false), ('^', 7, true, false),
    ('&', 8, true, false), ('*', 9, true, false), ('(', 10, true, false),
    (')', 11, true, false),
    ('-', 12, false, false), ('_', 12, true, false),
    ('=', 13, false, false), ('+', 13, true, false),
    ('[', 26, false, false), ('{', 26, true, false),
    (']', 27, false, false), ('}', 27, true, false),
    (';', 39, false, false), (':', 39, true, false),
    ('\'', 40, false, false), ('@', 40, true, false),
    ('`', 41, false, false),
    ('#', 43, false, false), ('~', 43, true, false),
    (',', 51, false, false), ('<', 51, true, false),
    ('.', 52, false, false), ('>', 52, true, false),
    ('/', 53, false, false), ('?', 53, true, false),
    ('\\', 86, false, false), ('|', 86, true, false),
];

// Layout code ("dk", "us", "uk", "fi") -> keymap. Anything else -> None.
fn keymap_for(code: &str) -> Option<Keymap> {
    match code {
        "dk" => Some(Keymap { name: "Danish", symbols: DK }),
        "fi" | "se" => Some(Keymap { name: "Finnish/Swedish", symbols: FI }),
        "gb" | "uk" => Some(Keymap { name: "English (UK)", symbols: UK }),
        "us" => Some(Keymap { name: "English (US)", symbols: US }),
        _ => None,
    }
}

fn parse_layout(v: &str) -> Keymap {
    let first = v.split(',').next().unwrap_or("").trim().to_lowercase();
    keymap_for(&first).unwrap_or(Keymap { name: "English (US)", symbols: US })
}

fn all_symbols() -> impl Iterator<Item = Sym> {
    US.iter().chain(UK).chain(FI).chain(DK).copied()
}

fn detect_keymap() -> Keymap {
    let home = std::env::var("HOME").unwrap_or_default();
    if let Ok(s) = std::fs::read_to_string(format!("{home}/.config/kxkbrc")) {
        for line in s.lines() {
            if let Some(v) = line.trim().strip_prefix("LayoutList=") {
                return parse_layout(v);
            }
        }
    }
    // Fallback (checked only once, since it starts a process)
    static FALLBACK: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let v = FALLBACK.get_or_init(|| {
        if let Ok(out) = std::process::Command::new("localectl").arg("status").output() {
            let t = String::from_utf8_lossy(&out.stdout).to_string();
            for line in t.lines() {
                if let Some(v) = line.trim().strip_prefix("X11 Layout:") {
                    return v.trim().to_string();
                }
            }
        }
        "us".to_string()
    });
    parse_layout(v)
}

// The saved setting is "auto" or a layout code ("dk", "us", "uk", "fi").
fn resolve(setting: &str) -> Keymap {
    keymap_for(setting).unwrap_or_else(detect_keymap)
}

fn bump(n: &mut i32, val: i32) {
    if val == 1 {
        *n += 1;
    } else if val == 0 {
        *n = (*n - 1).max(0);
    }
}

// A global shortcut such as "Ctrl+Alt+O"
struct Hotkey {
    ctrl: bool,
    alt: bool,
    shift: bool,
    meta: bool,
    key: u16,
}

fn key_code_for_name(name: &str) -> Option<u16> {
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return CHARS.iter().copied().find(|&(_, ch)| ch == c).map(|(k, _)| k);
    }
    let n: u16 = name.strip_prefix('f')?.parse().ok()?;
    match n {
        1..=10 => Some(58 + n),
        11 => Some(87),
        12 => Some(88),
        _ => None,
    }
}

fn parse_hotkey(s: &str) -> Option<Hotkey> {
    let mut hk = Hotkey {
        ctrl: false,
        alt: false,
        shift: false,
        meta: false,
        key: 0,
    };
    for part in s.split('+').map(|p| p.trim().to_lowercase()).filter(|p| !p.is_empty()) {
        match part.as_str() {
            "ctrl" | "control" => hk.ctrl = true,
            "alt" => hk.alt = true,
            "shift" => hk.shift = true,
            "meta" | "super" | "win" => hk.meta = true,
            other => hk.key = key_code_for_name(other)?,
        }
    }
    if hk.key == 0 {
        None
    } else {
        Some(hk)
    }
}

// Lets other parts of the program (voice typing) type text at the cursor
pub static TYPER: std::sync::OnceLock<std::sync::Mutex<mpsc::Sender<(usize, String, bool)>>> =
    std::sync::OnceLock::new();

pub fn type_text(text: String) -> bool {
    match TYPER.get() {
        Some(t) => t.lock().unwrap().send((0, text, true)).is_ok(),
        None => false,
    }
}

// How many modifier keys (Shift, Ctrl, Alt, AltGr, Meta) are held down on any keyboard right now
pub static GLOBAL_MODS: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

// Presses Ctrl+C for the user (used to copy the selected text). It first waits until the keys of
// the shortcut have been released, otherwise the program would see Ctrl+Alt+C or similar.
pub fn press_copy() -> bool {
    let began = std::time::Instant::now();
    while GLOBAL_MODS.load(Ordering::SeqCst) > 0 && began.elapsed() < Duration::from_millis(2000) {
        thread::sleep(Duration::from_millis(30));
    }
    thread::sleep(Duration::from_millis(60));
    match TYPER.get() {
        Some(t) => t
            .lock()
            .unwrap()
            .send((0, "\u{1}COPY".to_string(), false))
            .is_ok(),
        None => false,
    }
}

// An expansion waiting for the physical keys to be released before it is typed
struct Pending {
    key: u16,
    released: bool,
    job: (usize, String, bool),
}

fn tap(vd: &mut VirtualDevice, code: u16, shift: bool, altgr: bool) {
    let ev = |c: u16, v: i32| InputEvent::new(EventType::KEY, c, v);
    if shift {
        let _ = vd.emit(&[ev(LSHIFT, 1)]);
    }
    if altgr {
        let _ = vd.emit(&[ev(ALTGR, 1)]);
    }
    let _ = vd.emit(&[ev(code, 1)]);
    let _ = vd.emit(&[ev(code, 0)]);
    if altgr {
        let _ = vd.emit(&[ev(ALTGR, 0)]);
    }
    if shift {
        let _ = vd.emit(&[ev(LSHIFT, 0)]);
    }
    thread::sleep(Duration::from_millis(8));
}

pub fn start_listener(
    entries: Shared,
    enabled: Arc<AtomicBool>,
    layout: super::LayoutSetting,
    ocr: super::OcrShared,
    voice: super::VoiceShared,
    speak: super::SpeakShared,
) {
    let initial = resolve(&layout.lock().unwrap());
    eprintln!("Using keyboard layout: {}", initial.name);
    let km_shared = Arc::new(std::sync::Mutex::new(initial));

    // Follow layout changes (from the settings window) without a restart
    {
        let km_shared = km_shared.clone();
        thread::spawn(move || loop {
            thread::sleep(Duration::from_millis(1000));
            let setting = layout.lock().unwrap().clone();
            let new_km = resolve(&setting);
            let mut cur = km_shared.lock().unwrap();
            if cur.name != new_km.name {
                eprintln!("Switched keyboard layout to: {}", new_km.name);
                *cur = new_km;
            }
        });
    }

    let (tx, rx) = mpsc::channel::<(usize, String, bool)>();
    let _ = TYPER.set(std::sync::Mutex::new(tx.clone()));
    super::voice::start_idle_watch();
    super::speak::start_idle_watch();

    // Typing worker: virtual keyboard
    let km_worker = km_shared.clone();
    thread::spawn(move || {
        let mut keys = AttributeSet::<Key>::new();
        for (k, _) in CHARS {
            keys.insert(Key::new(*k));
        }
        // Register the keys of every layout so switching never needs a new device
        for (_, k, _, _) in all_symbols() {
            keys.insert(Key::new(k));
        }
        for k in [SPACE, BACKSPACE, LSHIFT, ALTGR, 29] {
            keys.insert(Key::new(k));
        }
        let built = VirtualDeviceBuilder::new()
            .and_then(|b| b.name(VNAME).with_keys(&keys))
            .and_then(|b| b.build());
        let mut vd = match built {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Could not create virtual keyboard (check /dev/uinput permissions): {e}");
                return;
            }
        };
        for (erase, text, trailing_space) in rx {
            let km = *km_worker.lock().unwrap();
            if text == "\u{1}COPY" {
                // Ctrl+C (left Ctrl = 29, C = 46)
                let ev = |c: u16, v: i32| InputEvent::new(EventType::KEY, c, v);
                let _ = vd.emit(&[ev(29, 1)]);
                tap(&mut vd, 46, false, false);
                let _ = vd.emit(&[ev(29, 0)]);
                continue;
            }
            thread::sleep(Duration::from_millis(30));
            for _ in 0..erase {
                tap(&mut vd, BACKSPACE, false, false);
            }
            for c in text.chars() {
                match km.keys_of(c) {
                    Some((k, s, a)) => tap(&mut vd, k, s, a),
                    None => eprintln!("Can't type character {c:?} with the {} layout", km.name),
                }
            }
            if trailing_space {
                tap(&mut vd, SPACE, false, false);
            }
        }
    });

    // One listener thread per physical keyboard
    let mut found = 0;
    for (_, mut dev) in evdev::enumerate() {
        let is_keyboard = dev
            .supported_keys()
            .map_or(false, |k| k.contains(Key::KEY_A) && k.contains(Key::KEY_SPACE));
        if !is_keyboard || dev.name() == Some(VNAME) {
            continue;
        }
        found += 1;
        let entries = entries.clone();
        let enabled = enabled.clone();
        let tx = tx.clone();
        let km_dev = km_shared.clone();
        let ocr_dev = ocr.clone();
        let voice_dev = voice.clone();
        let speak_dev = speak.clone();
        thread::spawn(move || {
            let mut buffer = String::new();
            let mut shift = 0i32;
            let mut altgr = 0i32;
            let mut ctrl = 0i32;
            let mut alt = 0i32;
            let mut meta = 0i32;
            let mut hk_cache: (String, Option<Hotkey>) = (String::new(), None);
            let mut vk_cache: (String, Option<Hotkey>) = (String::new(), None);
            let mut sk_cache: Vec<(String, Option<Hotkey>)> =
                (0..4).map(|_| (String::new(), None)).collect();
            let mut last_total_mods = 0i32;
            let mut pending: Option<Pending> = None;
            loop {
                let events = match dev.fetch_events() {
                    Ok(e) => e,
                    Err(_) => return,
                };
                for ev in events {
                    if ev.event_type() != EventType::KEY {
                        continue;
                    }
                    let (code, val) = (ev.code(), ev.value());
                    match code {
                        42 | 54 => bump(&mut shift, val),
                        ALTGR => bump(&mut altgr, val),
                        29 | 97 => bump(&mut ctrl, val),
                        56 => bump(&mut alt, val),
                        125 | 126 => bump(&mut meta, val),
                        _ => {}
                    }
                    let mods = ctrl + alt + meta;
                    let total_mods = shift + altgr + ctrl + alt + meta;
                    if total_mods != last_total_mods {
                        GLOBAL_MODS.fetch_add(total_mods - last_total_mods, Ordering::SeqCst);
                        last_total_mods = total_mods;
                    }
                    let is_modifier = matches!(code, 42 | 54 | ALTGR | 29 | 97 | 56 | 125 | 126);
                    if val == 0 {
                        // Hold-to-talk: letting go of the voice key stops the recording
                        if !is_modifier {
                            let vcfg = voice_dev.lock().unwrap().clone();
                            if vcfg.enabled && vcfg.mode == "hold" {
                                if vk_cache.0 != vcfg.shortcut {
                                    vk_cache = (vcfg.shortcut.clone(), parse_hotkey(&vcfg.shortcut));
                                }
                                let is_voice_key = match &vk_cache.1 {
                                    Some(hk) => hk.key == code,
                                    None => false,
                                };
                                if is_voice_key {
                                    thread::spawn(move || super::voice::stop_if_recording(vcfg));
                                }
                            }
                        }
                        // Wait until the physical trigger key AND any Shift/AltGr/Ctrl
                        // are released before typing, otherwise the system ignores or
                        // changes our virtual key presses.
                        if let Some(p) = pending.as_mut() {
                            if p.key == code {
                                p.released = true;
                            }
                        }
                        let ready = pending.as_ref().map_or(false, |p| p.released)
                            && shift == 0
                            && altgr == 0
                            && mods == 0;
                        if ready {
                            if let Some(p) = pending.take() {
                                thread::sleep(Duration::from_millis(20));
                                let _ = tx.send(p.job);
                            }
                        }
                        continue;
                    }
                    if is_modifier || val != 1 {
                        continue; // only real key presses from here on
                    }
                    // OCR shortcut: checked first, also while expansion is paused
                    {
                        let cfg = ocr_dev.lock().unwrap().clone();
                        if cfg.enabled {
                            if hk_cache.0 != cfg.shortcut {
                                hk_cache = (cfg.shortcut.clone(), parse_hotkey(&cfg.shortcut));
                            }
                            let hit = match &hk_cache.1 {
                                Some(hk) => {
                                    code == hk.key
                                        && (ctrl > 0) == hk.ctrl
                                        && (alt > 0) == hk.alt
                                        && (shift > 0) == hk.shift
                                        && (meta > 0) == hk.meta
                                        && altgr == 0
                                }
                                None => false,
                            };
                            if hit {
                                buffer.clear();
                                pending = None;
                                thread::spawn(move || super::ocr::run_with_notification(cfg));
                                continue;
                            }
                        }
                    }
                    // Voice typing shortcut
                    {
                        let vcfg = voice_dev.lock().unwrap().clone();
                        if vcfg.enabled {
                            if vk_cache.0 != vcfg.shortcut {
                                vk_cache = (vcfg.shortcut.clone(), parse_hotkey(&vcfg.shortcut));
                            }
                            let hit = match &vk_cache.1 {
                                Some(hk) => {
                                    code == hk.key
                                        && (ctrl > 0) == hk.ctrl
                                        && (alt > 0) == hk.alt
                                        && (shift > 0) == hk.shift
                                        && (meta > 0) == hk.meta
                                        && altgr == 0
                                }
                                None => false,
                            };
                            if hit {
                                buffer.clear();
                                pending = None;
                                thread::spawn(move || super::voice::toggle(vcfg));
                                continue;
                            }
                        }
                    }
                    // Read-aloud shortcuts
                    {
                        let scfg = speak_dev.lock().unwrap().clone();
                        if scfg.enabled {
                            let shortcuts: [String; 4] = [
                                scfg.read_da.clone(),
                                scfg.read_en.clone(),
                                scfg.stop.clone(),
                                scfg.pause.clone(),
                            ];
                            let actions = ["read-da", "read-en", "read-stop", "read-pause"];
                            let mut fired: Option<&'static str> = None;
                            for i in 0..4 {
                                if sk_cache[i].0 != shortcuts[i] {
                                    sk_cache[i] = (shortcuts[i].clone(), parse_hotkey(&shortcuts[i]));
                                }
                                let hit = match &sk_cache[i].1 {
                                    Some(hk) => {
                                        code == hk.key
                                            && (ctrl > 0) == hk.ctrl
                                            && (alt > 0) == hk.alt
                                            && (shift > 0) == hk.shift
                                            && (meta > 0) == hk.meta
                                            && altgr == 0
                                    }
                                    None => false,
                                };
                                if hit {
                                    fired = Some(actions[i]);
                                    break;
                                }
                            }
                            if let Some(action) = fired {
                                buffer.clear();
                                pending = None;
                                thread::spawn(move || super::speak::run_action(action, scfg));
                                continue;
                            }
                        }
                    }
                    if !enabled.load(Ordering::SeqCst) || mods > 0 {
                        buffer.clear();
                        pending = None;
                        continue;
                    }
                    if code != SPACE {
                        pending = None;
                    }
                    match code {
                        SPACE => {
                            let hit = entries
                                .lock()
                                .unwrap()
                                .iter()
                                .find(|e| e.space && e.trigger.to_lowercase() == buffer)
                                .map(|e| e.text.clone());
                            pending = hit.map(|text| Pending {
                                key: SPACE,
                                released: false,
                                job: (buffer.chars().count() + 1, text, true),
                            });
                            buffer.clear();
                        }
                        BACKSPACE => {
                            buffer.pop();
                        }
                        c => {
                            let km = *km_dev.lock().unwrap();
                            match km.char_of(c, shift > 0, altgr > 0) {
                                Some(ch) => {
                                    buffer.extend(ch.to_lowercase());
                                    if buffer.chars().count() > 32 {
                                        buffer.clear();
                                    } else {
                                        let hit = entries
                                            .lock()
                                            .unwrap()
                                            .iter()
                                            .find(|e| !e.space && e.trigger.to_lowercase() == buffer)
                                            .map(|e| e.text.clone());
                                        if let Some(text) = hit {
                                            pending = Some(Pending {
                                                key: code,
                                                released: false,
                                                job: (buffer.chars().count(), text, false),
                                            });
                                            buffer.clear();
                                        }
                                    }
                                }
                                None => buffer.clear(),
                            }
                        }
                    }
                }
            }
        });
    }
    if found == 0 {
        eprintln!("No readable keyboards found. Are you in the 'input' group (log out/in after adding)?");
    }
}
