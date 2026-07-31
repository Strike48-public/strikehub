#!/usr/bin/env bash
# Normalize raw clips to CFR 30fps bt709 full-range, then cut per-scene trim
# segments (from capture/flow.json, anchors resolved via <scene>.timings.json).
set -euo pipefail
shopt -s nullglob   # empty globs expand to nothing, not the literal pattern
cd "$(dirname "$0")"
FLOW=../capture/flow.json
mkdir -p scenes norm

# Resolve a trim anchor ("start" | "end" | "end-Ns" | "verify:<stepId>" | "+Ns" relative to prev) to seconds.
# Args: <scene> <anchor> <prevSeconds>
resolve_anchor() {
  local scene="$1" a="$2" prev="$3" t v raw
  local timings="${scene}.timings.json"
  local dur; dur=$(ffprobe -v error -show_entries format=duration -of default=nk=1:nw=1 "${scene}.mp4")
  case "$a" in
    start) raw=0 ;;
    end) raw="$dur" ;;
    end-*) v="${a#end-}"; v="${v%s}"; raw=$(awk "BEGIN{print $dur - $v}") ;;
    +*) v="${a#+}"; v="${v%s}"; raw=$(awk "BEGIN{print $prev + $v}") ;;
    verify:*) t=$(jq -r ".\"$a\" // empty" "$timings"); [ -n "$t" ] && raw="$t" || raw=0 ;;
    *) raw=0 ;;
  esac
  # Clamp to [0, dur]: verify timestamps are wall-clock and can land a hair past
  # the encoded duration when the recorder is stopped right at the last verify.
  awk "BEGIN{v=$raw; if(v<0)v=0; if(v>$dur)v=$dur; print v}"
}

# Force CFR + bt709 full-range on every cut.
cut() { # cut <src.mp4> <start> <end> <speed> <out.mp4>
  # Guard speed: setpts=(1/speed) divides by speed, so 0 or negative would make
  # ffmpeg abort the whole build. Default a bad/zero speed to 1.0.
  if awk "BEGIN{exit !($4 > 0)}"; then :; else
    echo "warn: non-positive speed '$4' for $5 — using 1.0"; set -- "$1" "$2" "$3" "1.0" "$5"
  fi
  ffmpeg -y -loglevel error -ss "$2" -to "$3" -i "$1" -an \
    -vf "setpts=(1/$4)*PTS,fps=30,scale=1920:1080:out_range=full:out_color_matrix=bt709,format=yuv420p" \
    -color_range pc -colorspace bt709 -color_primaries bt709 -color_trc bt709 \
    -c:v libx264 -preset fast -crf 20 "$5"
}

for scene in $(jq -r '.scenes[]|select(.record==true)|.id' "$FLOW"); do
  [ -f "${scene}.mp4" ] || { echo "skip ${scene}: no raw clip"; continue; }
  # Normalize to CFR first so setpts works. Keep it in RGB (libx264rgb) — the raw
  # clip is RGB (libx264rgb capture); a plain libx264 pass here would squeeze the
  # range back to limited before cut() does its single controlled RGB→bt709-full
  # conversion. Staying RGB until cut() preserves the deep blacks.
  ffmpeg -y -loglevel error -i "${scene}.mp4" -an -r 30 -vsync cfr \
    -vf "scale=1920:1080" -c:v libx264rgb -preset fast -qp 0 "norm/${scene}.mp4"
  n=0; prev=0
  seg_count=$(jq -r ".scenes[]|select(.id==\"$scene\")|.trim|length // 0" "$FLOW")
  if [ "$seg_count" = "0" ]; then
    cut "norm/${scene}.mp4" 0 "$(ffprobe -v error -show_entries format=duration -of default=nk=1:nw=1 norm/${scene}.mp4)" 1.0 "scenes/${scene}.mp4"
  else
    for i in $(seq 0 $((seg_count-1))); do
      fromA=$(jq -r ".scenes[]|select(.id==\"$scene\")|.trim[$i].from" "$FLOW")
      toA=$(jq -r ".scenes[]|select(.id==\"$scene\")|.trim[$i].to" "$FLOW")
      spd=$(jq -r ".scenes[]|select(.id==\"$scene\")|.trim[$i].speed" "$FLOW")
      from=$(resolve_anchor "$scene" "$fromA" "$prev")
      to=$(resolve_anchor "$scene" "$toA" "$from")
      # Skip degenerate segments (to <= from + 0.1s) — e.g. a trim anchored at the
      # final verify with no dwell footage after it — so ffmpeg never aborts.
      if awk "BEGIN{exit !($to > $from + 0.1)}"; then
        cut "norm/${scene}.mp4" "$from" "$to" "$spd" "scenes/${scene}_${n}.mp4"
        prev="$to"; n=$((n+1))
      else
        echo "skip ${scene} seg $i: empty range ($from..$to)"
        prev="$to"
      fi
    done
  fi
done

echo "=== scene clips ==="
for f in scenes/*.mp4; do echo "$f -> $(ffprobe -v error -show_entries format=duration -of default=nk=1:nw=1 "$f")s"; done
