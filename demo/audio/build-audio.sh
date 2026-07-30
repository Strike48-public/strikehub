#!/usr/bin/env bash
set -euo pipefail

# build-audio.sh — deterministic audio pipeline: script.json → Piper TTS → normalized wav files
# Produces demo/audio/vo/<scene>.wav for each scene + music bed at demo/audio/music.mp3

DEMO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
AUDIO_DIR="$DEMO_ROOT/audio"
SCRIPT="$AUDIO_DIR/script.json"
# ryan-high: Piper's high-quality US male voice — noticeably more natural than
# the lessac-medium model. ~120MB.
VOICE_MODEL="$AUDIO_DIR/voices/en_US-ryan-high.onnx"
VO_DIR="$AUDIO_DIR/vo"

if [[ ! -f "$VOICE_MODEL" ]]; then
  echo "ERROR: Voice model not found at $VOICE_MODEL"
  echo "Download it with:"
  echo "  curl -L -o $VOICE_MODEL https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/ryan/high/en_US-ryan-high.onnx"
  echo "  curl -L -o $VOICE_MODEL.json https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/ryan/high/en_US-ryan-high.onnx.json"
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
