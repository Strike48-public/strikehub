#!/usr/bin/env python3
"""Batch-synthesize voiceover lines with Kokoro-82M (local, Apache-2.0).

Usage: kokoro_tts.py <script.json> <out_dir> [voice]

Reads [{scene, vo}, ...] and writes <out_dir>/<scene>_raw.wav (24 kHz mono).
`vo` may contain Kokoro inline phoneme overrides like `[hub](/hʌb/)` to fix
mispronounced brand words. Deterministic-ish: fixed torch seed.

Requires a Python 3.12 venv (Kokoro caps at <3.13) with:
  pip install --prefer-binary "kokoro==0.9.4" "misaki[en]==0.9.4" soundfile torch
and LD_LIBRARY_PATH including libstdc++ (see build-audio.sh).
"""
import json
import os
import sys

import numpy as np
import soundfile as sf
import torch

torch.manual_seed(0)  # best-effort reproducibility

from kokoro import KPipeline  # noqa: E402 (import after seed)

script_path, out_dir = sys.argv[1], sys.argv[2]
voice = sys.argv[3] if len(sys.argv) > 3 else "bm_lewis"
# British voices need lang_code 'b'; American need 'a'.
lang = "b" if voice.startswith(("bm_", "bf_")) else "a"

pipeline = KPipeline(lang_code=lang)
os.makedirs(out_dir, exist_ok=True)

for entry in json.load(open(script_path)):
    scene, text = entry["scene"], entry["vo"]
    chunks = [np.asarray(audio) for _, _, audio in pipeline(text, voice=voice)]
    audio = np.concatenate(chunks)
    sf.write(f"{out_dir}/{scene}_raw.wav", audio, 24000)
    print(f"  -> {scene} ({round(len(audio) / 24000, 2)}s)", flush=True)
