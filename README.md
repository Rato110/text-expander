# Text Expander (Linux)

A small desktop helper for Linux, written in Rust. It runs in the background and adds four things you can use in any program:

| Tab | What it does | Default shortcut |
|---|---|---|
| **Text expander** | Type a short trigger (for example `omy`) and it becomes a longer text (`on my way`). Each trigger can need a Space or expand instantly. | – |
| **OCR** | Select an area of the screen and its text is copied to the clipboard. | `Ctrl+Alt+O` |
| **Voice typing** | Offline speech-to-text for **Danish and English**. Shows what you say in a live box, can type phrase by phrase, and can show an edit box so you can correct the text before it is typed. | `Ctrl+Alt+V` |
| **Read aloud** | Select text in any program and have it read out loud (Danish or English voice), with stop and pause/resume. | `Ctrl+Alt+D` (Danish), `Ctrl+Alt+E` (English), `Ctrl+Alt+S` (stop), `Ctrl+Alt+P` (pause/resume) |

Everything runs on your own computer. The settings window is available in English and Danish.

## Status

**Experimental.** It was developed and tested on **Fedora KDE Plasma (Wayland)** only. Other Linux desktops may work partly (some features use KDE's Klipper and Spectacle). OCR, voice typing and read aloud are Linux-only for now.

**Windows** support is on hold: the source has early, untested support for the text expander, but nothing is released for it. There is no macOS support.

Expect rough edges, and please open an issue if something breaks.

## Requirements

- Linux with a Wayland or X11 desktop (KDE Plasma recommended)
- `sudo` once, to install packages and a keyboard-access rule
- Internet during installation, and a few GB of free disk space (the Parakeet speech model is about 670 MB, each reading voice about 63 MB; the first build takes a few minutes)

## Install (AppImage, recommended)

The AppImage is one file that contains the program, the speech engines, a private Python (Parakeet speech and Piper voices), OCR with English and Danish, and the sound tools. You do not have to install packages.

1. Download `Text_Expander-<version>-x86_64.AppImage` from the **Releases** page (or from a finished run on the **Actions** tab, under *Artifacts*).
2. Make it executable and start it:

   ```bash
   chmod +x Text_Expander-*-x86_64.AppImage
   ./Text_Expander-*-x86_64.AppImage
   ```

   Some distributions need `libfuse2` to run AppImages. Without it, start the file with `--appimage-extract-and-run`.
3. The window opens on the **Setup** tab:
   - **Keyboard access**: press the button. It asks for your password once. Afterwards **log out and back in once**.
   - **Start at login** and **Application menu**: tick or press them (they point to the AppImage file, so keep it in a folder that does not change).
4. Then download what you want to use: a speech model in *Voice typing* (Parakeet) and voices in *Read aloud*.

Requirements: a 64-bit Intel/AMD computer with AVX2 (almost everything from 2014 on), glibc 2.35 or newer (Ubuntu 22.04, Fedora 36, Debian 12 or newer), Wayland or X11. Some features use helpers from your desktop: OCR needs a screenshot tool (Spectacle on KDE, gnome-screenshot, or grim + slurp), and reading selected text uses KDE's Klipper or wl-clipboard/xclip. The Setup tab shows what is missing on your computer.

## Install from source (Linux script)

```bash
git clone https://github.com/<your-name>/text-expander.git
cd text-expander
bash install.sh
```

The installer asks for your password when needed. It:

1. installs build tools and helpers (Rust, a C++ compiler, cmake, Tesseract, wl-clipboard, sound tools, Python) with `dnf`, `apt` or `pacman`;
2. builds the program and puts it in `~/.local/bin`;
3. builds the offline speech engines (whisper.cpp) and sets up a private Python for the fast Parakeet engine and the Piper voices;
4. adds you to the `input` group and a `uinput` rule, so the program can see global shortcuts and type text on Wayland;
5. creates a menu entry, entries for KDE shortcuts, and a systemd user service so it starts at login.

**Log out and back in once** after the first install (the `input` group needs it). Then open **Text Expander** from your application menu.

First use:
- *Voice typing*: open the tab and press **Download** on the first model (Parakeet).
- *Read aloud*: open the tab and download one Danish and one English voice.
- *OCR*: works as soon as the packages are installed.

The window title shows the version, for example `Text Expander v0.16.0`.

## Windows (on hold)

Windows support is paused while the Linux AppImage is finished. The notes below describe the early state: only the text expander and starting at login exist there. The Windows build is no longer run automatically; start the **build** workflow by hand from the Actions tab if you want to try it.

1. On GitHub open the **Actions** tab, click the latest successful run of **build**, and download **text-expander-windows** under *Artifacts*. (You must be logged in to GitHub.)
2. Unzip it and put `expander.exe` in a folder of your choice, for example `C:\Users\<you>\TextExpander`.
3. Double-click `expander.exe`. Windows SmartScreen may say the program is unknown, because it is not signed: choose **More info → Run anyway**. Security software can also be wary of programs that watch the keyboard; add an exception if needed.
4. The settings window opens and the expander starts in the background. Add a trigger, tick **Start automatically when I log in** if you like, and try it in any program.

Settings are stored in `%APPDATA%\expander`. Windows does not let a normal program type into windows that run as administrator. To build it yourself you need Rust with the Visual Studio C++ build tools, then `cargo build --release`.

## Shortcuts that type a letter

The program watches the keyboard but cannot hide a key from other programs. If a shortcut such as `Ctrl+Alt+D` types a letter into your document:

- use a key that types nothing, for example `F9`, or
- let KDE handle the shortcut: **System Settings → Keyboard → Shortcuts → Add Command**, with one of:

```
~/.local/bin/expander --voice-toggle     # start/stop voice typing
~/.local/bin/expander --voice-start
~/.local/bin/expander --voice-stop
~/.local/bin/expander --ocr
~/.local/bin/expander --read-da          # read the selection in Danish
~/.local/bin/expander --read-en          # read the selection in English
~/.local/bin/expander --read-stop
~/.local/bin/expander --read-pause
```

## Project layout

```
src/main.rs          settings, background service, the settings window
src/input_linux.rs   keyboard watching and typing on Linux (evdev / uinput)
src/input_other.rs   keyboard watching and typing on Windows and macOS (rdev / enigo)
src/voice.rs         voice typing (Linux)
src/speak.rs         read aloud (Linux)
src/ocr.rs           OCR (Linux)
src/control.rs       commands for the desktop's own shortcuts (Linux)
src/setup.rs         one-time setup: keyboard access, start at login, menu entries (Linux)
packaging/           the AppImage: AppRun, desktop file, icon, build script
install.sh           Linux installer (from source)
scripts/             helper scripts for the Python parts
```

## Privacy and security

- Speech recognition, OCR and the reading voices run locally. The only network use is downloading: the program source and models from GitHub and Hugging Face, and Python packages from PyPI.
- To support global shortcuts and trigger words on Wayland, the program reads keyboard events from `/dev/input` and types through a virtual keyboard (`/dev/uinput`). It keeps only a short buffer of recent characters to match triggers and does not store what you type. Be aware that membership of the `input` group lets *any* program you run read keyboard events, so only install this if you are comfortable with that. The code is here to be inspected.

## Files and logs

- Settings and logs: `~/.config/expander/` (`triggers.json`, `voice.json`, `ocr.json`, `speak.json`, `expander.log`, `parakeet-server.log`, `tts-server.log`)
- Models, voices and the private Python: `~/.local/share/expander/`
- Python setup log: `~/.local/share/expander/python-setup.log`; build log: `build.log`

If the Python setup fails, run `scripts/setup-fast-engine.sh` (speech) and `scripts/setup-reading-voice.sh` (voices) and read the log it prints. The fast speech engine needs Python 3.10–3.14; if your system Python is newer, the script downloads a private Python 3.13.

## Uninstall

```bash
systemctl --user disable --now expander.service
rm -f ~/.local/bin/expander ~/.config/systemd/user/expander.service
rm -f ~/.local/share/applications/expander*.desktop
rm -rf ~/.config/expander ~/.local/share/expander
sudo rm -f /etc/udev/rules.d/99-uinput.rules /etc/modules-load.d/uinput.conf
sudo gpasswd -d "$USER" input
```

## Credits and licenses

This project is licensed under the MIT License (see `LICENSE`). It downloads and uses other software and models, each under its own license. As far as I know, and please check each project's page before you redistribute anything:

- [whisper.cpp](https://github.com/ggml-org/whisper.cpp) (MIT) – speech engine for the Whisper models
- NVIDIA Parakeet TDT 0.6B v3 (see its model card; used through [onnx-asr](https://github.com/istupakov/onnx-asr) and the GGUF files from `ggml-org/parakeet-GGUF`)
- [Piper](https://github.com/OHF-Voice/piper1-gpl) (GPL, installed as a separate Python package and run as a separate process) and the Piper voices from `rhasspy/piper-voices` (each voice has its own license on its model card)
- [Tesseract](https://github.com/tesseract-ocr/tesseract) (Apache-2.0) for OCR
- Rust crates such as `egui`/`eframe`, `rdev`, `enigo`, `evdev`, `serde`

The design of the voice typing (offline Parakeet, a live box, typing at pauses) was inspired by [TuxTalk](https://github.com/ronb1964/TuxTalk). No TuxTalk code was used.

This project was developed with the help of an AI assistant (Claude, by Anthropic).
