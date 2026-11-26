#!/usr/bin/env bash
# Sets up only the Python part of the fast Parakeet engine (quick; no root needed).
# Run it with:  bash setup-fast-engine.sh
set -uo pipefail
echo "==> Setting up the fast Parakeet engine (Python)"
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

echo
if "$PYENV/bin/python" -c "import onnx_asr, onnxruntime, numpy, huggingface_hub" >/dev/null 2>&1; then
  echo "Done. Close and reopen Text Expander, then press Download on the first model in the Voice typing tab."
else
  echo "The setup did not finish. Please send Claude the log lines above, and the output of:  python3 --version"
  echo "(The full log is in $PYLOG)"
fi
