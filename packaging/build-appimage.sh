#!/usr/bin/env bash
# Builds the AppImage: the program, the speech engines, a private Python (Parakeet speech + Piper
# voices), Tesseract (OCR, English + Danish) and the sound tools, all in one file.
#
# Meant for Ubuntu 22.04 (the AppImage then runs on systems with glibc 2.35 or newer), for example
# the GitHub Actions runner. Needs: cmake, git, curl, python3-pip, patchelf, tesseract-ocr with the
# eng + dan language packs and pulseaudio-utils (see .github/workflows/build.yml).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORK="${WORK:-$ROOT/build-appimage}"
APPDIR="$WORK/AppDir"
VERSION="$(grep -m1 '^version' "$ROOT/Cargo.toml" | cut -d'"' -f2)"
OUT="$ROOT/Text_Expander-$VERSION-x86_64.AppImage"
export APPIMAGE_EXTRACT_AND_RUN=1   # AppImages used as tools here run without FUSE

rm -rf "$APPDIR"
mkdir -p "$WORK" "$APPDIR/usr/bin" "$APPDIR/usr/share/tessdata" \
         "$APPDIR/usr/share/icons/hicolor/256x256/apps" "$APPDIR/usr/share/applications"

echo "==> 1/7 The program"
( cd "$ROOT" && cargo build --release )
cp "$ROOT/target/release/expander" "$APPDIR/usr/bin/expander"

echo "==> 2/7 Speech engines (whisper.cpp: Whisper and Parakeet)"
if [ ! -d "$WORK/whisper.cpp" ]; then
  git clone --depth 1 https://github.com/ggml-org/whisper.cpp "$WORK/whisper.cpp"
fi
# portable settings: no "-march=native", so that the AppImage runs on other computers
# (it needs a processor with AVX2, which almost everything from 2014 on has)
cmake -S "$WORK/whisper.cpp" -B "$WORK/whisper.cpp/build" -DCMAKE_BUILD_TYPE=Release \
  -DBUILD_SHARED_LIBS=OFF -DGGML_NATIVE=OFF -DGGML_AVX=ON -DGGML_AVX2=ON -DGGML_FMA=ON -DGGML_F16C=ON \
  -DCMAKE_EXE_LINKER_FLAGS="-static-libstdc++ -static-libgcc"
cmake --build "$WORK/whisper.cpp/build" -j"$(nproc)" --config Release --target whisper-cli
cp "$WORK/whisper.cpp/build/bin/whisper-cli" "$APPDIR/usr/bin/whisper-cli"
if cmake --build "$WORK/whisper.cpp/build" -j"$(nproc)" --config Release --target parakeet-cli; then
  cp "$WORK/whisper.cpp/build/bin/parakeet-cli" "$APPDIR/usr/bin/parakeet-cli"
else
  echo "NOTE: this whisper.cpp has no parakeet-cli; the fast Python Parakeet engine is used instead."
fi

echo "==> 3/7 Python with the Parakeet (onnx-asr) and Piper packages"
python3 -m pip install --user --upgrade uv
export PATH="$HOME/.local/bin:$PATH"
export UV_PYTHON_INSTALL_DIR="$WORK/python-install"
rm -rf "$UV_PYTHON_INSTALL_DIR"
uv python install 3.13
PYHOME="$(find "$UV_PYTHON_INSTALL_DIR" -maxdepth 1 -mindepth 1 -type d -name 'cpython-3.13*' | head -n 1)"
[ -n "$PYHOME" ] || { echo "Could not find the downloaded Python"; exit 1; }
uv pip install --python "$PYHOME/bin/python3" --break-system-packages "onnx-asr[cpu,hub]" piper-tts
rm -rf "$APPDIR/usr/python"
cp -a "$PYHOME" "$APPDIR/usr/python"
# smaller: no byte-code caches and no Python test suite
find "$APPDIR/usr/python" -type d -name '__pycache__' -prune -exec rm -rf {} + 2>/dev/null || true
rm -rf "$APPDIR"/usr/python/lib/python3.13/test 2>/dev/null || true

echo "==> 4/7 Tesseract (OCR) with English and Danish"
cp "$(command -v tesseract)" "$APPDIR/usr/bin/tesseract"
for lang in eng dan osd; do
  FILE="$(find /usr/share -name "$lang.traineddata" 2>/dev/null | head -n 1)"
  [ -n "$FILE" ] || { echo "Missing the Tesseract language data: $lang"; exit 1; }
  cp "$FILE" "$APPDIR/usr/share/tessdata/"
done

echo "==> 5/7 Sound tools (recording and playing)"
cp "$(command -v parec)" "$APPDIR/usr/bin/parec"
cp "$(command -v paplay)" "$APPDIR/usr/bin/paplay"

echo "==> 6/7 Icon and menu entry"
python3 "$ROOT/packaging/make-icon.py" "$APPDIR/usr/share/icons/hicolor/256x256/apps/expander.png"
cp "$ROOT/packaging/expander.desktop" "$APPDIR/usr/share/applications/expander.desktop"

echo "==> 7/7 Libraries and the AppImage file"
cd "$WORK"
[ -f linuxdeploy-x86_64.AppImage ] || curl -fsSL -o linuxdeploy-x86_64.AppImage \
  https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage
[ -f appimagetool-x86_64.AppImage ] || curl -fsSL -o appimagetool-x86_64.AppImage \
  https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
chmod +x linuxdeploy-x86_64.AppImage appimagetool-x86_64.AppImage

EXES=(-e "$APPDIR/usr/bin/expander" -e "$APPDIR/usr/bin/whisper-cli" -e "$APPDIR/usr/bin/tesseract" \
      -e "$APPDIR/usr/bin/parec" -e "$APPDIR/usr/bin/paplay")
[ -f "$APPDIR/usr/bin/parakeet-cli" ] && EXES+=(-e "$APPDIR/usr/bin/parakeet-cli")
ARCH=x86_64 ./linuxdeploy-x86_64.AppImage --appdir "$APPDIR" "${EXES[@]}" \
  --custom-apprun "$ROOT/packaging/AppRun" \
  --desktop-file "$ROOT/packaging/expander.desktop" \
  --icon-file "$APPDIR/usr/share/icons/hicolor/256x256/apps/expander.png"

echo "==> Quick checks of what is inside"
TESSDATA_PREFIX="$APPDIR/usr/share/tessdata" "$APPDIR/usr/bin/tesseract" --list-langs
PYTHONNOUSERSITE=1 "$APPDIR/usr/python/bin/python3" -c "import onnx_asr, onnxruntime, numpy, huggingface_hub, piper; print('Python packages OK')"
"$APPDIR/usr/bin/whisper-cli" -h >/dev/null 2>&1 && echo "whisper-cli OK" || echo "WARNING: whisper-cli did not start"

ARCH=x86_64 ./appimagetool-x86_64.AppImage --no-appstream "$APPDIR" "$OUT"
echo
echo "Done: $OUT"
ls -lh "$OUT"
