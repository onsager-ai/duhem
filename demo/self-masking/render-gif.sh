#!/usr/bin/env bash
# Renders demo.svg's animation into demo.gif — for surfaces that don't
# animate SVG (e.g. the npm package page). Needs a Chromium and an
# ffmpeg; CHROME / FFMPEG default to Playwright's bundled binaries.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
CHROME="${CHROME:-chromium}"
# Needs a FULL ffmpeg (gif muxer + concat demuxer + palettegen). Note:
# Playwright's bundled ffmpeg is too stripped for GIF — use a system one.
FFMPEG="${FFMPEG:-ffmpeg}"
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT

# Reveal times (ms) — must track render-svg.mjs — and how long to hold each.
times=(0   600 1200 2100 2800 3000 3200 3400 4300 5100 5700 6400)
holds=(0.6 0.6 0.9  0.7  0.2  0.2  0.2  0.9  0.8  0.6  0.7  2.4)

: > "$tmp/list.txt"
for i in "${!times[@]}"; do
  node "$here/render-svg.mjs" --at "${times[$i]}" > "$tmp/f$i.svg"
  "$CHROME" --headless --no-sandbox --disable-gpu --hide-scrollbars \
    --force-device-scale-factor=2 --window-size=600,360 \
    --screenshot="$tmp/f$i.png" "file://$tmp/f$i.svg" >/dev/null 2>&1
  printf "file '%s'\nduration %s\n" "$tmp/f$i.png" "${holds[$i]}" >> "$tmp/list.txt"
done
# concat demuxer honors a frame's duration only if the frame is repeated after it
last=$((${#times[@]} - 1)); printf "file '%s'\n" "$tmp/f$last.png" >> "$tmp/list.txt"

"$FFMPEG" -y -f concat -safe 0 -i "$tmp/list.txt" \
  -vf "scale=680:-1:flags=lanczos,split[a][b];[a]palettegen=stats_mode=full[p];[b][p]paletteuse=dither=bayer" \
  -loop 0 "$here/demo.gif" >/dev/null 2>&1
echo "wrote demo.gif ($(du -h "$here/demo.gif" | cut -f1))"
