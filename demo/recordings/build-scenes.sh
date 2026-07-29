#!/usr/bin/env bash
# Normalize raw clips to CFR 30fps bt709 full-range, then cut per-scene trim
# segments (from capture/flow.json, anchors resolved via <scene>.timings.json).
set -euo pipefail
cd "$(dirname "$0")"
FLOW=../capture/flow.json
mkdir -p scenes norm

# Resolve a trim anchor ("start" | "end" | "end-Ns" | "verify:<stepId>" | "+Ns" relative to prev) to seconds.
# Args: <scene> <anchor> <prevSeconds>
resolve_anchor() {
  local scene="$1" a="$2" prev="$3" t v
  local timings="${scene}.timings.json"
  local dur; dur=$(ffprobe -v error -show_entries format=duration -of default=nk=1:nw=1 "${scene}.mp4")
  case "$a" in
    start) echo 0 ;;
    end) echo "$dur" ;;
    end-*) v="${a#end-}"; v="${v%s}"; awk "BEGIN{print $dur - $v}" ;;
    +*) v="${a#+}"; v="${v%s}"; awk "BEGIN{print $prev + $v}" ;;
    verify:*) t=$(jq -r ".\"$a\" // empty" "$timings"); [ -n "$t" ] && echo "$t" || echo 0 ;;
    *) echo 0 ;;
  esac
}

# Force CFR + bt709 full-range on every cut.
cut() { # cut <src.mp4> <start> <end> <speed> <out.mp4>
  ffmpeg -y -loglevel error -ss "$2" -to "$3" -i "$1" -an \
    -vf "setpts=(1/$4)*PTS,fps=30,scale=1920:1080:out_range=full:out_color_matrix=bt709,format=yuv420p" \
    -color_range pc -colorspace bt709 -color_primaries bt709 -color_trc bt709 \
    -c:v libx264 -preset fast -crf 20 "$5"
}

for scene in $(jq -r '.scenes[]|select(.record==true)|.id' "$FLOW"); do
  [ -f "${scene}.mp4" ] || { echo "skip ${scene}: no raw clip"; continue; }
  # normalize to CFR first so setpts works
  ffmpeg -y -loglevel error -i "${scene}.mp4" -an -r 30 -vsync cfr \
    -vf "scale=1920:1080" -c:v libx264 -preset fast -crf 20 "norm/${scene}.mp4"
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
      cut "norm/${scene}.mp4" "$from" "$to" "$spd" "scenes/${scene}_${n}.mp4"
      prev="$to"; n=$((n+1))
    done
  fi
done

echo "=== scene clips ==="
for f in scenes/*.mp4; do echo "$f -> $(ffprobe -v error -show_entries format=duration -of default=nk=1:nw=1 "$f")s"; done
