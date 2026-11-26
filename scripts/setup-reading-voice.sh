#!/usr/bin/env bash
# Sets up only the read-aloud voice engine (Piper). Quick; no root needed.
# Run it with:  bash setup-reading-voice.sh   (after setup-fast-engine.sh has worked)
set -uo pipefail
echo "==> Setting up the read-aloud voice engine (Piper)"
PYENV="$HOME/.local/share/expander/py"
PYLOG="$HOME/.local/share/expander/python-setup.log"
mkdir -p "$(dirname "$PYLOG")"
if [ ! -x "$PYENV/bin/python" ]; then
  echo "The Python environment is missing, so the read-aloud engine could not be set up. Run setup-fast-engine.sh first."
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

echo
if [ -x "$PYENV/bin/python" ] && "$PYENV/bin/python" -c "from piper import PiperVoice" >/dev/null 2>&1; then
  echo "Done. Close and reopen Text Expander, open the Read aloud tab and download a voice."
else
  echo "The setup did not finish. Please send Claude the log lines above."
fi
