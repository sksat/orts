#!/usr/bin/env bash
# Downloads and prepares high-resolution Earth textures from NASA public domain imagery.
#
# Day:   NASA SVS 3615 Blue Marble (with clouds), 8192x4096
# Night: NASA Black Marble 2016 color maps
#
# Prerequisites: curl, imagemagick (convert or magick)
# Usage: ./tools/fetch-textures.sh [--resolution 4k|8k|all] [--force]

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TEXTURE_DIR="$SCRIPT_DIR/../viewer/public/textures"
TEMP_DIR=$(mktemp -d)
trap 'rm -rf "$TEMP_DIR"' EXIT

# --- NASA source URLs (public domain) ---

# Blue Marble flat equirectangular with clouds (SVS 3615)
DAY_8K_URL="https://svs.gsfc.nasa.gov/vis/a000000/a003600/a003615/flat_earth_Largest_still.0330.jpg"

# Black Marble 2016 color maps
NIGHT_LOW_URL="https://assets.science.nasa.gov/content/dam/science/esd/eo/images/imagerecords/144000/144898/BlackMarble_2016_01deg.jpg"   # 3600x1800
NIGHT_HIGH_URL="https://assets.science.nasa.gov/content/dam/science/esd/eo/images/imagerecords/144000/144898/BlackMarble_2016_3km.jpg"  # 13500x6750

# --- Parse arguments ---

RESOLUTION="all"
FORCE=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --resolution) RESOLUTION="$2"; shift 2 ;;
    --force)      FORCE=true; shift ;;
    -h|--help)
      echo "Usage: $0 [--resolution 4k|8k|all] [--force]"
      echo ""
      echo "Downloads NASA Earth textures and resizes to power-of-two dimensions."
      echo "Output: viewer/public/textures/earth_{4k,8k}.jpg, earth_night_{4k,8k}.jpg"
      echo ""
      echo "Options:"
      echo "  --resolution  Which resolutions to download: 4k, 8k, or all (default: all)"
      echo "  --force       Re-download even if files already exist"
      exit 0
      ;;
    *) echo "Unknown option: $1"; exit 1 ;;
  esac
done

# --- Check prerequisites ---

if ! command -v curl &>/dev/null; then
  echo "Error: curl is required but not installed." >&2
  exit 1
fi

MAGICK_CMD=""
if command -v magick &>/dev/null; then
  MAGICK_CMD="magick"
elif command -v convert &>/dev/null; then
  MAGICK_CMD="convert"
else
  echo "Error: ImageMagick (magick or convert) is required but not installed." >&2
  exit 1
fi

# --- Helper functions ---

download() {
  local url="$1" dest="$2"
  echo "  Downloading $(basename "$dest")..."
  curl -fSL --progress-bar -o "$dest" "$url"
}

resize_jpeg() {
  local src="$1" dest="$2" width="$3" height="$4"
  echo "  Resizing to ${width}x${height}..."
  $MAGICK_CMD "$src" -resize "${width}x${height}!" -quality 90 "$dest"
}

should_process() {
  local file="$1"
  if [[ "$FORCE" == true ]]; then return 0; fi
  if [[ -f "$file" ]]; then
    echo "  $(basename "$file") already exists (use --force to re-download)"
    return 1
  fi
  return 0
}

# --- Download and process ---

mkdir -p "$TEXTURE_DIR"

# Day textures (from 8K source)
process_day() {
  local src_8k="$TEMP_DIR/day_source_8k.jpg"

  # Download source only if we need it
  local need_download=false
  if [[ "$RESOLUTION" == "4k" || "$RESOLUTION" == "all" ]] && should_process "$TEXTURE_DIR/earth_4k.jpg"; then
    need_download=true
  fi
  if [[ "$RESOLUTION" == "8k" || "$RESOLUTION" == "all" ]] && should_process "$TEXTURE_DIR/earth_8k.jpg"; then
    need_download=true
  fi

  if [[ "$need_download" == false ]]; then return; fi

  echo "==> Fetching Blue Marble day texture (8192x4096)..."
  download "$DAY_8K_URL" "$src_8k"

  if [[ "$RESOLUTION" == "8k" || "$RESOLUTION" == "all" ]] && should_process "$TEXTURE_DIR/earth_8k.jpg"; then
    echo "==> Creating earth_8k.jpg..."
    # Source is already 8192x4096, just optimize
    $MAGICK_CMD "$src_8k" -quality 90 "$TEXTURE_DIR/earth_8k.jpg"
    echo "  Done: $(du -h "$TEXTURE_DIR/earth_8k.jpg" | cut -f1)"
  fi

  if [[ "$RESOLUTION" == "4k" || "$RESOLUTION" == "all" ]] && should_process "$TEXTURE_DIR/earth_4k.jpg"; then
    echo "==> Creating earth_4k.jpg..."
    resize_jpeg "$src_8k" "$TEXTURE_DIR/earth_4k.jpg" 4096 2048
    echo "  Done: $(du -h "$TEXTURE_DIR/earth_4k.jpg" | cut -f1)"
  fi
}

# Night textures (from two different sources for 4K and 8K)
process_night() {
  if [[ "$RESOLUTION" == "4k" || "$RESOLUTION" == "all" ]] && should_process "$TEXTURE_DIR/earth_night_4k.jpg"; then
    echo "==> Fetching Black Marble night texture (3600x1800)..."
    local src_low="$TEMP_DIR/night_source_low.jpg"
    download "$NIGHT_LOW_URL" "$src_low"
    echo "==> Creating earth_night_4k.jpg..."
    resize_jpeg "$src_low" "$TEXTURE_DIR/earth_night_4k.jpg" 4096 2048
    echo "  Done: $(du -h "$TEXTURE_DIR/earth_night_4k.jpg" | cut -f1)"
  fi

  if [[ "$RESOLUTION" == "8k" || "$RESOLUTION" == "all" ]] && should_process "$TEXTURE_DIR/earth_night_8k.jpg"; then
    echo "==> Fetching Black Marble night texture (13500x6750)..."
    local src_high="$TEMP_DIR/night_source_high.jpg"
    download "$NIGHT_HIGH_URL" "$src_high"
    echo "==> Creating earth_night_8k.jpg..."
    resize_jpeg "$src_high" "$TEXTURE_DIR/earth_night_8k.jpg" 8192 4096
    echo "  Done: $(du -h "$TEXTURE_DIR/earth_night_8k.jpg" | cut -f1)"
  fi
}

process_day
process_night

echo ""
echo "==> All done! Texture files in $TEXTURE_DIR:"
ls -lh "$TEXTURE_DIR"/earth*.jpg
