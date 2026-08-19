#!/usr/bin/env bash
set -euo pipefail

# build-audio.sh — deterministic audio pipeline: script.json → Kokoro-82M TTS
#   → normalized wav files. Produces demo/audio/vo/<scene>.wav + music bed.
#
# Voice: Kokoro bm_lewis (British male narrator). Kokoro is Apache-2.0, local,
# CPU-fine, and far more natural than Piper. It requires Python 3.12 (upstream
# caps at <3.13) and libstdc++ on LD_LIBRARY_PATH; we build a cached venv below.
# script.json may embed inline phoneme overrides, e.g. "Strike [hʌb](/hʌb/)",
# to fix brand-word pronunciation.

DEMO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
AUDIO_DIR="$DEMO_ROOT/audio"
SCRIPT="$AUDIO_DIR/script.json"
VO_DIR="$AUDIO_DIR/vo"
VOICE="${KOKORO_VOICE:-bm_lewis}"
VENV="${KOKORO_VENV:-$AUDIO_DIR/.kokoro-venv}"   # cached; gitignored

mkdir -p "$VO_DIR"

echo "==> Ensuring Kokoro venv ($VENV)..."
# Create the py3.12 venv once (prebuilt wheels only; torch from the CPU index).
if [[ ! -x "$VENV/bin/python" ]]; then
  nix-shell -p python312 stdenv.cc.cc.lib zlib --run "
    set -euo pipefail
    export LD_LIBRARY_PATH=\"\$(cc --print-file-name=libstdc++.so.6 | xargs dirname):\$(dirname \$(ls /nix/store/*zlib*/lib/libz.so.1 2>/dev/null | head -1)):\${LD_LIBRARY_PATH:-}\"
    python3 -m venv '$VENV'
    . '$VENV/bin/activate'
    pip -q install --upgrade pip
    pip -q install torch --index-url https://download.pytorch.org/whl/cpu
    pip -q install --prefer-binary 'kokoro==0.9.4' 'misaki[en]==0.9.4' soundfile numpy
  "
fi

echo "==> Synthesizing voiceover ($VOICE) from $SCRIPT..."
nix-shell -p python312 stdenv.cc.cc.lib zlib espeak-ng ffmpeg jq --run "
  set -euo pipefail
  export LD_LIBRARY_PATH=\"\$(cc --print-file-name=libstdc++.so.6 | xargs dirname):\$(dirname \$(ls /nix/store/*zlib*/lib/libz.so.1 2>/dev/null | head -1)):\${LD_LIBRARY_PATH:-}\"
  '$VENV/bin/python' '$AUDIO_DIR/kokoro_tts.py' '$SCRIPT' '$VO_DIR' '$VOICE'
  # Normalize each raw 24 kHz wav → 16-bit PCM 44.1 kHz mono, loudnorm.
  jq -r '.[].scene' '$SCRIPT' | while read -r scene; do
    ffmpeg -y -i \"$VO_DIR/\${scene}_raw.wav\" -ar 44100 -ac 1 -sample_fmt s16 \
      -filter:a 'loudnorm=I=-16:LRA=11:TP=-1.5' \"$VO_DIR/\${scene}.wav\" >/dev/null 2>&1
    rm \"$VO_DIR/\${scene}_raw.wav\"
    dur=\$(ffprobe -v error -show_entries format=duration -of default=nk=1:nw=1 \"$VO_DIR/\${scene}.wav\" 2>/dev/null)
    echo \"     ✓ \${scene}.wav (\${dur}s)\"
  done
"

echo "==> Preparing music bed..."
# Synthesized ambient pad (our own output — CC0-equivalent, no licensing issue).
# A sustained Cadd9 chord (C3 G3 C4 E4 G4 D5) with tremolo shimmer + echo/reverb
# and a soft low-pass, normalized to an audible bed level. Regenerated every run
# so it's deterministic. Swap in a licensed track here if a richer bed is wanted.
MUSIC_OUT="$AUDIO_DIR/music.mp3"
nix-shell -p ffmpeg --run "
  ffmpeg -y -loglevel error \
    -f lavfi -i 'sine=frequency=130.81:duration=70' \
    -f lavfi -i 'sine=frequency=196.00:duration=70' \
    -f lavfi -i 'sine=frequency=261.63:duration=70' \
    -f lavfi -i 'sine=frequency=329.63:duration=70' \
    -f lavfi -i 'sine=frequency=392.00:duration=70' \
    -f lavfi -i 'sine=frequency=587.33:duration=70' \
    -filter_complex '[0][1][2][3][4][5]amix=inputs=6:normalize=1,tremolo=f=0.13:d=0.45,vibrato=f=0.15:d=0.2,aecho=0.8:0.85:900|1600:0.35|0.25,lowpass=f=1700,highpass=f=70,loudnorm=I=-20:TP=-2,afade=t=in:st=0:d=3,afade=t=out:st=67:d=3,aformat=channel_layouts=stereo:sample_rates=44100' \
    -t 70 -b:a 160k '$MUSIC_OUT'
"
echo "  ✓ $MUSIC_OUT (synthesized ambient pad)"

echo "==> Audio build complete!"
echo "    Voiceover files: $VO_DIR/*.wav"
echo "    Music bed: $MUSIC_OUT"
