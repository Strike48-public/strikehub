#!/usr/bin/env bash
set -euo pipefail

# build-audio.sh — deterministic audio pipeline: script.json → Piper TTS → normalized wav files
# Produces demo/audio/vo/<scene>.wav for each scene + music bed at demo/audio/music.mp3

DEMO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
AUDIO_DIR="$DEMO_ROOT/audio"
SCRIPT="$AUDIO_DIR/script.json"
VOICE_MODEL="$AUDIO_DIR/voices/en_US-lessac-medium.onnx"
VO_DIR="$AUDIO_DIR/vo"

if [[ ! -f "$VOICE_MODEL" ]]; then
  echo "ERROR: Voice model not found at $VOICE_MODEL"
  echo "Download it with:"
  echo "  curl -L -o $VOICE_MODEL https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/medium/en_US-lessac-medium.onnx"
  echo "  curl -L -o $VOICE_MODEL.json https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/medium/en_US-lessac-medium.onnx.json"
  exit 1
fi

mkdir -p "$VO_DIR"

echo "==> Synthesizing voiceover from $SCRIPT..."
# Parse script.json and generate a wav for each scene
nix-shell -p piper-tts jq ffmpeg --run "
  set -euo pipefail
  jq -c '.[]' '$SCRIPT' | while read -r entry; do
    scene=\$(echo \"\$entry\" | jq -r '.scene')
    text=\$(echo \"\$entry\" | jq -r '.vo')
    out_raw=\"$VO_DIR/\${scene}_raw.wav\"
    out_final=\"$VO_DIR/\${scene}.wav\"

    echo \"  → \$scene\"
    echo \"\$text\" | piper --model '$VOICE_MODEL' --output_file \"\$out_raw\" 2>&1 | grep -v '^$' || true

    # Normalize audio: convert to 16-bit PCM 44.1kHz mono, apply light normalization
    ffmpeg -y -i \"\$out_raw\" -ar 44100 -ac 1 -sample_fmt s16 -filter:a 'loudnorm=I=-16:LRA=11:TP=-1.5' \"\$out_final\" >/dev/null 2>&1
    rm \"\$out_raw\"

    duration=\$(ffprobe -v error -show_entries format=duration -of default=noprint_wrappers=1:nokey=1 \"\$out_final\" 2>/dev/null)
    echo \"     ✓ \$out_final (\${duration}s)\"
  done
"

echo "==> Preparing music bed..."
# For now, generate a simple silent placeholder — integrator can replace with real music
# A real implementation would download/generate a CC0 instrumental bed here
MUSIC_OUT="$AUDIO_DIR/music.mp3"
if [[ ! -f "$MUSIC_OUT" ]]; then
  nix-shell -p ffmpeg --run "
    ffmpeg -y -f lavfi -i 'anoisesrc=color=white:r=44100:d=90' \
      -filter:a 'lowpass=f=200,highpass=f=80,volume=0.02' \
      -t 90 -b:a 128k '$MUSIC_OUT' >/dev/null 2>&1
  "
  echo "  ✓ $MUSIC_OUT (generated 90s ambient bed — replace with real track)"
else
  echo "  ✓ $MUSIC_OUT (exists)"
fi

echo "==> Audio build complete!"
echo "    Voiceover files: $VO_DIR/*.wav"
echo "    Music bed: $MUSIC_OUT"
