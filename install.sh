#!/usr/bin/env bash
# Text Expander installer for Linux (Wayland-friendly, uses evdev + uinput).
# Usage, from the repository folder:  bash install.sh
set -euo pipefail

if [ "$(uname -s)" != "Linux" ]; then
  echo "This installer is for Linux only." >&2
  exit 1
fi

if [ "$(id -u)" -eq 0 ]; then
  echo "Run this as your normal user, not root (it will ask for sudo when needed)." >&2
  exit 1
fi

APP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"   # the folder of this repository
BIN_DIR="$HOME/.local/bin"

echo "==> 1/6 Installing build dependencies"
if command -v dnf >/dev/null 2>&1; then
  sudo dnf install -y gcc pkgconf-pkg-config libxkbcommon-devel mesa-libGL-devel \
    libxcb-devel wayland-devel libX11-devel libXcursor-devel libXrandr-devel libXi-devel curl \
    tesseract tesseract-langpack-dan wl-clipboard xclip libnotify \
    cmake gcc-c++ git pulseaudio-utils python3 python3-pip
elif command -v apt >/dev/null 2>&1; then
  sudo apt update
  sudo apt install -y build-essential pkg-config libxkbcommon-dev libgl1-mesa-dev \
    libwayland-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev libxcb1-dev curl \
    tesseract-ocr tesseract-ocr-dan wl-clipboard xclip libnotify-bin \
    cmake g++ git pulseaudio-utils python3 python3-pip python3-venv
elif command -v pacman >/dev/null 2>&1; then
  sudo pacman -S --needed --noconfirm base-devel pkgconf libxkbcommon mesa wayland \
    libx11 libxcursor libxrandr libxi libxcb curl \
    tesseract tesseract-data-eng tesseract-data-dan wl-clipboard xclip libnotify \
    cmake git libpulse python python-pip
else
  echo "Unknown package manager. Install a C compiler, pkg-config, libxkbcommon, GL and X11/Wayland dev packages manually." >&2
fi

echo "==> 2/6 Installing Rust (if needed)"
if ! command -v cargo >/dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
fi
# shellcheck disable=SC1091
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

echo "==> 4/6 Building (the first build takes a few minutes)"
echo "Building version $(grep -m1 '^version' "$APP_DIR/Cargo.toml" | cut -d'"' -f2) ..."
if ! ( cd "$APP_DIR" && cargo build --release 2>&1 | tee "$APP_DIR/build.log" ); then
  echo
  echo "=================================================================="
  echo " THE BUILD FAILED."
  echo " Please open an issue on GitHub and paste the error lines below."
  echo "=================================================================="
  grep -n -A14 "^error" "$APP_DIR/build.log" | head -80
  echo "------------------------------------------------------------------"
  echo " (The full log is saved in $APP_DIR/build.log)"
  exit 1
fi

# ---- put the new program in place
mkdir -p "$BIN_DIR"
# Stop any running copy first (a running program can not be overwritten)
systemctl --user stop expander.service 2>/dev/null || true
pkill -x expander 2>/dev/null || true
sleep 1
cp "$APP_DIR/target/release/expander" "$BIN_DIR/expander"
if cmp -s "$APP_DIR/target/release/expander" "$BIN_DIR/expander"; then
  echo "Installed the new program: $BIN_DIR/expander"
else
  echo "ERROR: the new program could not be copied to $BIN_DIR/expander" >&2
  exit 1
fi

echo "==> Building the offline voice engines (whisper.cpp: Whisper and Parakeet). This takes a few minutes the first time."
WHISPER_SRC="$HOME/.local/share/expander/whisper-src"
WHISPER_DIR="$HOME/.local/share/expander/whisper"
mkdir -p "$WHISPER_DIR"
if [ -x "$WHISPER_DIR/whisper-cli" ] && [ -f "$WHISPER_DIR/.built-v2" ]; then
  echo "Voice engines already installed."
else
  (
    set -e
    rm -rf "$WHISPER_SRC"
    git clone --depth 1 https://github.com/ggml-org/whisper.cpp "$WHISPER_SRC"
    cmake -S "$WHISPER_SRC" -B "$WHISPER_SRC/build" -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF
    cmake --build "$WHISPER_SRC/build" -j"$(nproc)" --config Release --target whisper-cli
    cp "$WHISPER_SRC/build/bin/whisper-cli" "$WHISPER_DIR/whisper-cli"
    # Parakeet (fast, supports Danish and English). Not fatal if this version of whisper.cpp lacks it.
    if cmake --build "$WHISPER_SRC/build" -j"$(nproc)" --config Release --target parakeet-cli; then
      cp "$WHISPER_SRC/build/bin/parakeet-cli" "$WHISPER_DIR/parakeet-cli"
    else
      echo "Could not build the Parakeet engine. The Whisper models still work."
    fi
    touch "$WHISPER_DIR/.built-v2"
  ) && echo "Voice engines installed." || echo "Could not build the voice engines. The rest of Text Expander is fine; run this installer again later to retry."
  rm -rf "$WHISPER_SRC" 2>/dev/null || true
fi

echo "==> Setting up the fast Parakeet engine (Python). The model itself is downloaded later from the app."
PYENV="$HOME/.local/share/expander/py"
PYLOG="$HOME/.local/share/expander/python-setup.log"
mkdir -p "$(dirname "$PYLOG")"
if [ -x "$PYENV/bin/python" ] && "$PYENV/bin/python" -c "import onnx_asr, onnxruntime, numpy, huggingface_hub" >/dev/null 2>&1; then
  echo "Fast Parakeet engine already set up."
else
  : > "$PYLOG"
  PY_OK=0
  # 1) a Python that is already installed (the speech libraries support Python 3.10 to 3.14)
  for PYBIN in python3.13 python3.12 python3.11 python3.14 python3.10 python3; do
    command -v "$PYBIN" >/dev/null 2>&1 || continue
    PYNUM=$("$PYBIN" -c 'import sys; print(sys.version_info[0]*100+sys.version_info[1])' 2>/dev/null) || continue
    if [ "$PYNUM" -lt 310 ] || [ "$PYNUM" -gt 314 ]; then
      echo "Skipping $PYBIN ($("$PYBIN" --version 2>&1)): the speech libraries need Python 3.10 to 3.14." | tee -a "$PYLOG"
      continue
    fi
    echo "Trying $PYBIN ($("$PYBIN" --version 2>&1)) ..." | tee -a "$PYLOG"
    rm -rf "$PYENV"
    "$PYBIN" -m venv "$PYENV" >>"$PYLOG" 2>&1 || continue
    "$PYENV/bin/pip" install --upgrade pip >>"$PYLOG" 2>&1 || true
    if "$PYENV/bin/pip" install "onnx-asr[cpu,hub]" >>"$PYLOG" 2>&1; then
      PY_OK=1
    elif "$PYENV/bin/pip" install numpy onnxruntime huggingface-hub >>"$PYLOG" 2>&1 \
         && "$PYENV/bin/pip" install --no-deps onnx-asr >>"$PYLOG" 2>&1; then
      PY_OK=1
    fi
    if [ "$PY_OK" = "1" ]; then
      # onnxruntime 1.24.1 is known not to work with onnx-asr: use another version
      if [ "$("$PYENV/bin/python" -c 'import onnxruntime; print(onnxruntime.__version__)' 2>/dev/null)" = "1.24.1" ]; then
        "$PYENV/bin/pip" install "onnxruntime!=1.24.1" >>"$PYLOG" 2>&1 || true
      fi
      if "$PYENV/bin/python" -c "import onnx_asr, onnxruntime, numpy, huggingface_hub" >>"$PYLOG" 2>&1; then
        break
      fi
    fi
    echo "That did not work with $PYBIN." | tee -a "$PYLOG"
    PY_OK=0
  done

  # 2) no suitable Python on this computer: get a private copy of Python 3.13 with "uv" (no root needed)
  if [ "$PY_OK" != "1" ]; then
    echo "No suitable Python found. Getting a private copy of Python 3.13 (a small download) ..." | tee -a "$PYLOG"
    UVENV="$HOME/.local/share/expander/uv-tool"
    rm -rf "$UVENV"
    export UV_PYTHON_INSTALL_DIR="$HOME/.local/share/expander/python"
    if python3 -m venv "$UVENV" >>"$PYLOG" 2>&1 && "$UVENV/bin/pip" install uv >>"$PYLOG" 2>&1; then
      if "$UVENV/bin/uv" python install 3.13 >>"$PYLOG" 2>&1 \
         && rm -rf "$PYENV" \
         && "$UVENV/bin/uv" venv --python 3.13 "$PYENV" >>"$PYLOG" 2>&1 \
         && "$UVENV/bin/uv" pip install --python "$PYENV/bin/python" "onnx-asr[cpu,hub]" >>"$PYLOG" 2>&1 \
         && "$PYENV/bin/python" -c "import onnx_asr, onnxruntime, numpy, huggingface_hub" >>"$PYLOG" 2>&1; then
        PY_OK=1
      fi
    fi
  fi

  if [ "$PY_OK" = "1" ]; then
    echo "Fast Parakeet engine set up."
  else
    echo "Could not set up the fast Parakeet engine. The other voice engines still work."
    echo "Last lines of the log ($PYLOG):"
    tail -n 30 "$PYLOG"
  fi
fi

echo "==> Setting up the read-aloud voice engine (Piper)"
PYENV="$HOME/.local/share/expander/py"
PYLOG="$HOME/.local/share/expander/python-setup.log"
mkdir -p "$(dirname "$PYLOG")"
if [ ! -x "$PYENV/bin/python" ]; then
  echo "The Python environment is missing, so the read-aloud engine could not be set up. Run scripts/setup-fast-engine.sh first."
elif "$PYENV/bin/python" -c "from piper import PiperVoice" >/dev/null 2>&1; then
  echo "Read-aloud voice engine already set up."
else
  UVBIN="$HOME/.local/share/expander/uv-tool/bin/uv"
  PIPER_OK=0
  if "$PYENV/bin/python" -m pip --version >/dev/null 2>&1; then
    "$PYENV/bin/python" -m pip install piper-tts >>"$PYLOG" 2>&1 && PIPER_OK=1
  fi
  if [ "$PIPER_OK" != "1" ] && [ -x "$UVBIN" ]; then
    "$UVBIN" pip install --python "$PYENV/bin/python" piper-tts >>"$PYLOG" 2>&1 && PIPER_OK=1
  fi
  if [ "$PIPER_OK" != "1" ]; then
    "$PYENV/bin/python" -m ensurepip --upgrade >>"$PYLOG" 2>&1 \
      && "$PYENV/bin/python" -m pip install piper-tts >>"$PYLOG" 2>&1 && PIPER_OK=1
  fi
  if [ "$PIPER_OK" = "1" ] && "$PYENV/bin/python" -c "from piper import PiperVoice" >>"$PYLOG" 2>&1; then
    echo "Read-aloud voice engine set up."
  else
    echo "Could not set up the read-aloud voice engine (Piper). Everything else still works."
    echo "Last lines of the log ($PYLOG):"
    tail -n 25 "$PYLOG"
  fi
fi

echo "==> 5/6 Setting up keyboard access (input group + uinput)"
sudo usermod -aG input "$USER"
echo uinput | sudo tee /etc/modules-load.d/uinput.conf >/dev/null
sudo modprobe uinput || true
echo 'KERNEL=="uinput", GROUP="input", MODE="0660"' | sudo tee /etc/udev/rules.d/99-uinput.rules >/dev/null
sudo udevadm control --reload
sudo udevadm trigger

echo "==> 6/6 Creating app menu entry and background autostart"
mkdir -p "$HOME/.local/share/applications"
cat > "$HOME/.local/share/applications/expander.desktop" <<DESKTOP_EOF
cat > "$HOME/.local/share/applications/expander-voice.desktop" <<VOICE_EOF
[Desktop Entry]
Type=Application
Name=Text Expander: voice typing (start/stop)
Comment=Start or stop voice typing. Assign a shortcut to this in System Settings > Keyboard > Shortcuts.
Exec=$BIN_DIR/expander --voice-toggle
Terminal=false
Categories=Utility;
VOICE_EOF
cat > "$HOME/.local/share/applications/expander-ocr.desktop" <<OCR_EOF
cat > "$HOME/.local/share/applications/expander-read-da.desktop" <<READ_DA_EOF
[Desktop Entry]
Type=Application
Name=Text Expander: read aloud in Danish
Comment=Read the selected text aloud in Danish. Assign a shortcut to this in System Settings > Keyboard > Shortcuts.
Exec=$BIN_DIR/expander --read-da
Terminal=false
Categories=Utility;
READ_DA_EOF
cat > "$HOME/.local/share/applications/expander-read-en.desktop" <<READ_EN_EOF
[Desktop Entry]
Type=Application
Name=Text Expander: read aloud in English
Comment=Read the selected text aloud in English. Assign a shortcut to this in System Settings > Keyboard > Shortcuts.
Exec=$BIN_DIR/expander --read-en
Terminal=false
Categories=Utility;
READ_EN_EOF
cat > "$HOME/.local/share/applications/expander-read-stop.desktop" <<READ_STOP_EOF
[Desktop Entry]
Type=Application
Name=Text Expander: stop reading
Comment=Stop reading aloud. Assign a shortcut to this in System Settings > Keyboard > Shortcuts.
Exec=$BIN_DIR/expander --read-stop
Terminal=false
Categories=Utility;
READ_STOP_EOF
cat > "$HOME/.local/share/applications/expander-read-pause.desktop" <<READ_PAUSE_EOF
[Desktop Entry]
Type=Application
Name=Text Expander: pause / resume reading
Comment=Pause or resume reading aloud. Assign a shortcut to this in System Settings > Keyboard > Shortcuts.
Exec=$BIN_DIR/expander --read-pause
Terminal=false
Categories=Utility;
READ_PAUSE_EOF
[Desktop Entry]
Type=Application
Name=Text Expander: read text from the screen (OCR)
Comment=Select an area and copy its text. Assign a shortcut to this in System Settings > Keyboard > Shortcuts.
Exec=$BIN_DIR/expander --ocr
Terminal=false
Categories=Utility;
OCR_EOF
[Desktop Entry]
Type=Application
Name=Text Expander
Comment=Edit your text expander triggers
Exec=$BIN_DIR/expander
Terminal=false
Categories=Utility;
DESKTOP_EOF

# Background service: a systemd user service. systemd force-stops it within
# 2 seconds at shutdown/restart, so it can never hold up your PC.
mkdir -p "$HOME/.config/systemd/user"
cat > "$HOME/.config/systemd/user/expander.service" <<SERVICE_EOF
[Unit]
Description=Text Expander background service

[Service]
ExecStart=$BIN_DIR/expander --daemon
Restart=on-failure
KillSignal=SIGKILL
TimeoutStopSec=2

[Install]
WantedBy=default.target
SERVICE_EOF

# The old login-autostart entry would start a second copy (double expansion), so remove it
rm -f "$HOME/.config/autostart/expander.desktop"
systemctl --user daemon-reload

ans="y"
read -r -p "Start the expander in the background automatically at login? [Y/n] " ans || ans="y"
if [[ "$ans" =~ ^[Nn]$ ]]; then
  systemctl --user disable expander.service 2>/dev/null || true
  echo "Autostart disabled. Start it from the settings window when you want it."
else
  systemctl --user enable expander.service
  echo "Autostart enabled (runs in the background, no window)."
  # Start it right away if keyboard access is already active (you were in the input group before)
  if id -nG | tr ' ' '\n' | grep -qx input; then
    systemctl --user restart expander.service || true
    echo "Background expander started."
  fi
fi

echo
echo "Done."
echo "If this is your first install, log out and back in once so the 'input' group takes effect."
echo "Open the settings window from the app menu (Text Expander) or run: expander"
echo "OCR: open the OCR tab in the window to set the shortcut (default Ctrl+Alt+O)."
echo "Voice typing: open the Voice typing tab, press Download on the first model (Parakeet, fastest), then use the shortcut (default Ctrl+Alt+V)."
echo "If a shortcut types a letter into your document: use F9, or set the shortcut in KDE (see the hint in the Voice typing tab)."
echo "Read aloud: open the Read aloud tab, download a Danish and an English voice, then select text and use the shortcuts (default Ctrl+Alt+D / E / S / P)."
echo "Closing the window does NOT stop the expander; use the Stop button in the window to stop it."
echo "Your triggers are saved in ~/.config/expander/triggers.json"
