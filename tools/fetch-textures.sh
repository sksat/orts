#!/usr/bin/env bash
# Downloads and prepares textures from NASA/USGS public domain imagery.
#
# All textures are US government works (public domain, no copyright).
#
# Earth Day:   NASA Blue Marble Next Generation w/ Topography and Bathymetry (cloud-free)
# Earth Night: NASA Black Marble 2016 color maps
# Moon:        NASA CGI Moon Kit 2019 (LRO WAC mosaic)
# Mars:        USGS Viking MDIM 2.1 Colorized Global Mosaic
# Sun:         NASA SDO/STEREO EUV 304Å Carrington map
#
# Prerequisites: curl, imagemagick (convert or magick)
# Usage: ./tools/fetch-textures.sh [--resolution 2k,4k,8k,16k|all] [--body earth,moon,mars,sun|all] [--force]

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TEXTURE_DIR="$SCRIPT_DIR/../viewer/public/textures"
TEMP_DIR=$(mktemp -d)
trap 'rm -rf "$TEMP_DIR"' EXIT

# --- NASA source URLs (public domain) ---

# Blue Marble Next Generation w/ Topography and Bathymetry – cloud-free (Visible Earth 73909)
# December 2004, 21600x10800 equirectangular
# Source: https://visibleearth.nasa.gov/images/73909
DAY_SOURCE_URL="https://eoimages.gsfc.nasa.gov/images/imagerecords/73000/73909/world.topo.bathy.200412.3x21600x10800.jpg"  # 21600x10800

# Black Marble 2016 color maps
NIGHT_LOW_URL="https://assets.science.nasa.gov/content/dam/science/esd/eo/images/imagerecords/144000/144898/BlackMarble_2016_01deg.jpg"   # 3600x1800
NIGHT_HIGH_URL="https://assets.science.nasa.gov/content/dam/science/esd/eo/images/imagerecords/144000/144898/BlackMarble_2016_3km.jpg"  # 13500x6750

# NASA CGI Moon Kit 2019 — LRO WAC color mosaic (equirectangular)
# Source: https://svs.gsfc.nasa.gov/4720/
MOON_2K_URL="https://svs.gsfc.nasa.gov/vis/a000000/a004700/a004720/lroc_color_2k.jpg"              # 2048x1024
MOON_4K_URL="https://svs.gsfc.nasa.gov/vis/a000000/a004700/a004720/lroc_color_poles_4k.tif"         # 4096x2048
MOON_8K_URL="https://svs.gsfc.nasa.gov/vis/a000000/a004700/a004720/lroc_color_poles_8k.tif"         # 8192x4096
MOON_16K_URL="https://svs.gsfc.nasa.gov/vis/a000000/a004700/a004720/lroc_color_poles_16k.tif"       # 16384x8192

# USGS Viking MDIM 2.1 Colorized Global Mosaic — 232m/pixel (equirectangular)
# Source: https://astrogeology.usgs.gov/search/map/mars_viking_colorized_global_mosaic_232m
MARS_PREVIEW_URL="https://astrogeology.usgs.gov/ckan/dataset/7131d503-cdc9-45a5-8f83-5126c0fd397e/resource/6afad901-1caa-48a7-8b62-3911da0004c2/download/mars_viking_mdim21_clrmosaic_global_1024.jpg"  # 1024x512
MARS_SOURCE_URL="https://planetarymaps.usgs.gov/mosaic/Mars_Viking_MDIM21_ClrMosaic_global_232m.tif"  # 92160x46080, 12GB

# NASA SDO/STEREO EUV 304Å Carrington map (equirectangular)
# Source: https://svs.gsfc.nasa.gov/30362/
SUN_SOURCE_URL="https://svs.gsfc.nasa.gov/vis/a030000/a030300/a030362/euvi_aia304_2012_carrington.tif"  # 4104x2304

# --- Parse arguments ---

RESOLUTIONS="all"
BODIES="all"
FORCE=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --resolution) [[ $# -ge 2 ]] || { echo "Error: --resolution requires a value" >&2; exit 1; }; RESOLUTIONS="$2"; shift 2 ;;
    --body)       [[ $# -ge 2 ]] || { echo "Error: --body requires a value" >&2; exit 1; }; BODIES="$2"; shift 2 ;;
    --force)      FORCE=true; shift ;;
    -h|--help)
      echo "Usage: $0 [--resolution 2k,4k,8k,16k|all] [--body earth,moon,mars,sun|all] [--force]"
      echo ""
      echo "Downloads NASA/USGS textures and converts to power-of-two JPEG."
      echo ""
      echo "Output:"
      echo "  Earth:  earth_{2k,4k,8k,16k}.jpg, earth_night_{2k,4k,8k,16k}.jpg"
      echo "  Moon:   moon.jpg (2k), moon_{4k,8k,16k}.jpg"
      echo "  Mars:   mars.jpg (2k), mars_{4k,8k,16k}.jpg"
      echo "  Sun:    sun.jpg (2k), sun_4k.jpg"
      echo ""
      echo "Options:"
      echo "  --resolution  Comma-separated list of resolutions: 2k,4k,8k,16k or all (default: all)"
      echo "                One invocation covering every resolution you want downloads each"
      echo "                source once; separate invocations re-download it per resolution"
      echo "  --body        Comma-separated list of bodies: earth,moon,mars,sun or all (default: all)"
      echo "                Tip: omit mars to skip the 12GB source download required for mars 4k/8k/16k"
      echo "  --force       Re-download even if files already exist"
      exit 0
      ;;
    *) echo "Unknown option: $1"; exit 1 ;;
  esac
done

# Validates a comma-separated selection and writes it back to the named variable
# lowercased and space-stripped, so --body "Earth, Moon" works as expected and
# include_body / include_resolution stay plain list membership tests.
normalise_selection() {
  local var="$1" flag="$2" allowed="$3"
  local norm; norm=$(tr '[:upper:]' '[:lower:]' <<< "${!var}")
  norm="${norm// /}"
  # An empty selection would otherwise pass validation and produce nothing while
  # exiting 0, hiding an unset variable in whatever called this.
  [[ -n "$norm" ]] || { echo "Error: --$flag needs a value (expected: $allowed or all)" >&2; exit 1; }
  local entry entries
  IFS=',' read -ra entries <<< "$norm"
  for entry in "${entries[@]}"; do
    [[ ",$allowed,all," == *",$entry,"* ]] || {
      echo "Error: unknown $flag '$entry' (expected: $allowed or all)" >&2; exit 1
    }
  done
  if [[ "$norm" != "all" ]] && [[ ",$norm," == *",all,"* ]]; then
    echo "Error: 'all' cannot be combined with specific values; use '--$flag all' alone" >&2; exit 1
  fi
  printf -v "$var" '%s' "$norm"
}

normalise_selection RESOLUTIONS resolution "2k,4k,8k,16k"
normalise_selection BODIES body "earth,moon,mars,sun"

# Returns 0 if the given body should be processed.
include_body() {
  [[ "$BODIES" == "all" ]] || [[ ",$BODIES," == *",$1,"* ]]
}

# Returns 0 if the given resolution should be produced.
include_resolution() {
  [[ "$RESOLUTIONS" == "all" ]] || [[ ",$RESOLUTIONS," == *",$1,"* ]]
}

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

# Retry budget for the NASA/USGS hosts, which drop connections mid-transfer and
# go unreachable for minutes at a time. A deploy that missed the texture cache
# has no second source, so a single bad transfer must not end the run.
CURL_RETRIES=5
CURL_RETRY_DELAY=5
CURL_CONNECT_TIMEOUT=30      # curl defaults to 300s; an unreachable host would eat the budget on one attempt
CURL_STALL_BYTES_PER_SEC=1024
CURL_STALL_SECONDS=60        # under 1 KiB/s for a minute the transfer is stuck, not slow — fail it and let a retry resume

download() {
  local url="$1" dest="$2"
  local tmp="$TEMP_DIR/$(basename "$dest").partial"
  echo "  Downloading $(basename "$dest")..."
  # --retry-all-errors is what makes the retries reachable: curl treats only
  # timeouts and a few HTTP codes as transient, so a truncated transfer
  # (exit 18, seen on the 21600x10800 Blue Marble source) would otherwise fail
  # on the first attempt without retrying once. -C - makes curl's own retries
  # resume: a retry after a cut connection sends Range from what is already on
  # disk, and every source here answers that with 206.
  # No --retry-max-time: its timer starts before the first attempt, so a limit
  # shorter than the transfer itself (the Blue Marble source takes minutes on a
  # slow link, the Mars source far longer) expires before the failure it is
  # meant to cover, leaving no retry at all. Attempts are bounded individually
  # by the connect timeout and the stall detector instead.
  curl -fSL \
    --retry "$CURL_RETRIES" --retry-delay "$CURL_RETRY_DELAY" --retry-all-errors \
    --connect-timeout "$CURL_CONNECT_TIMEOUT" \
    --speed-limit "$CURL_STALL_BYTES_PER_SEC" --speed-time "$CURL_STALL_SECONDS" \
    -C - --progress-bar -o "$tmp" "$url"
  mv "$tmp" "$dest"
}

resize_jpeg() {
  local src="$1" dest="$2" width="$3" height="$4"
  echo "  Resizing to ${width}x${height}..."
  $MAGICK_CMD "$src" -resize "${width}x${height}!" -quality 90 "$dest"
}

# Convert any image (TIFF, etc.) to JPEG at original resolution
convert_jpeg() {
  local src="$1" dest="$2"
  echo "  Converting to JPEG..."
  $MAGICK_CMD "$src" -quality 90 "$dest"
}

# Convert and resize any image to JPEG
convert_resize_jpeg() {
  local src="$1" dest="$2" width="$3" height="$4"
  echo "  Converting and resizing to ${width}x${height}..."
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

# Day textures (from 21600x10800 cloud-free source)
process_day() {
  local src="$TEMP_DIR/day_source.jpg"

  # Download source only if we need it
  local need_download=false
  for res in 2k 4k 8k 16k; do
    if include_resolution "$res" && should_process "$TEXTURE_DIR/earth_${res}.jpg"; then
      need_download=true
    fi
  done

  if [[ "$need_download" == false ]]; then return; fi

  echo "==> Fetching Blue Marble cloud-free day texture (21600x10800)..."
  download "$DAY_SOURCE_URL" "$src"

  if include_resolution 16k && should_process "$TEXTURE_DIR/earth_16k.jpg"; then
    echo "==> Creating earth_16k.jpg..."
    resize_jpeg "$src" "$TEXTURE_DIR/earth_16k.jpg" 16384 8192
    echo "  Done: $(du -h "$TEXTURE_DIR/earth_16k.jpg" | cut -f1)"
  fi

  if include_resolution 8k && should_process "$TEXTURE_DIR/earth_8k.jpg"; then
    echo "==> Creating earth_8k.jpg..."
    resize_jpeg "$src" "$TEXTURE_DIR/earth_8k.jpg" 8192 4096
    echo "  Done: $(du -h "$TEXTURE_DIR/earth_8k.jpg" | cut -f1)"
  fi

  if include_resolution 4k && should_process "$TEXTURE_DIR/earth_4k.jpg"; then
    echo "==> Creating earth_4k.jpg..."
    resize_jpeg "$src" "$TEXTURE_DIR/earth_4k.jpg" 4096 2048
    echo "  Done: $(du -h "$TEXTURE_DIR/earth_4k.jpg" | cut -f1)"
  fi

  if include_resolution 2k && should_process "$TEXTURE_DIR/earth_2k.jpg"; then
    echo "==> Creating earth_2k.jpg..."
    resize_jpeg "$src" "$TEXTURE_DIR/earth_2k.jpg" 2048 1024
    echo "  Done: $(du -h "$TEXTURE_DIR/earth_2k.jpg" | cut -f1)"
  fi
}

# Night textures (from two different sources for 4K and 8K)
process_night() {
  if include_resolution 4k && should_process "$TEXTURE_DIR/earth_night_4k.jpg"; then
    echo "==> Fetching Black Marble night texture (3600x1800)..."
    local src_low="$TEMP_DIR/night_source_low.jpg"
    download "$NIGHT_LOW_URL" "$src_low"
    echo "==> Creating earth_night_4k.jpg..."
    resize_jpeg "$src_low" "$TEXTURE_DIR/earth_night_4k.jpg" 4096 2048
    echo "  Done: $(du -h "$TEXTURE_DIR/earth_night_4k.jpg" | cut -f1)"
  fi

  if include_resolution 8k || include_resolution 16k; then
    local src_high="$TEMP_DIR/night_source_high.jpg"
    local need_high=false

    # Only a requested resolution justifies the 13500x6750 source: without the
    # include_resolution guard, --resolution 8k downloaded it whenever the 16k
    # output happened to be missing, which it is on every fresh checkout.
    if include_resolution 8k && should_process "$TEXTURE_DIR/earth_night_8k.jpg"; then need_high=true; fi
    if include_resolution 16k && should_process "$TEXTURE_DIR/earth_night_16k.jpg"; then need_high=true; fi

    if [[ "$need_high" == true ]]; then
      echo "==> Fetching Black Marble night texture (13500x6750)..."
      download "$NIGHT_HIGH_URL" "$src_high"

      if include_resolution 16k && should_process "$TEXTURE_DIR/earth_night_16k.jpg"; then
        echo "==> Creating earth_night_16k.jpg..."
        resize_jpeg "$src_high" "$TEXTURE_DIR/earth_night_16k.jpg" 16384 8192
        echo "  Done: $(du -h "$TEXTURE_DIR/earth_night_16k.jpg" | cut -f1)"
      fi

      if include_resolution 8k && should_process "$TEXTURE_DIR/earth_night_8k.jpg"; then
        echo "==> Creating earth_night_8k.jpg..."
        resize_jpeg "$src_high" "$TEXTURE_DIR/earth_night_8k.jpg" 8192 4096
        echo "  Done: $(du -h "$TEXTURE_DIR/earth_night_8k.jpg" | cut -f1)"
      fi
    fi
  fi
}

# Moon textures (NASA CGI Moon Kit — pre-rendered per resolution)
process_moon() {
  # 2k: direct JPEG download
  if include_resolution 2k && should_process "$TEXTURE_DIR/moon.jpg"; then
    echo "==> Fetching Moon 2K texture (LRO WAC mosaic)..."
    download "$MOON_2K_URL" "$TEXTURE_DIR/moon.jpg"
    echo "  Done: $(du -h "$TEXTURE_DIR/moon.jpg" | cut -f1)"
  fi

  # 4k/8k/16k: TIFF → JPEG conversion (no resize needed, NASA provides exact sizes)
  if include_resolution 4k && should_process "$TEXTURE_DIR/moon_4k.jpg"; then
    echo "==> Fetching Moon 4K texture..."
    local src="$TEMP_DIR/moon_4k.tif"
    download "$MOON_4K_URL" "$src"
    echo "==> Creating moon_4k.jpg..."
    convert_jpeg "$src" "$TEXTURE_DIR/moon_4k.jpg"
    echo "  Done: $(du -h "$TEXTURE_DIR/moon_4k.jpg" | cut -f1)"
  fi

  if include_resolution 8k && should_process "$TEXTURE_DIR/moon_8k.jpg"; then
    echo "==> Fetching Moon 8K texture..."
    local src="$TEMP_DIR/moon_8k.tif"
    download "$MOON_8K_URL" "$src"
    echo "==> Creating moon_8k.jpg..."
    convert_jpeg "$src" "$TEXTURE_DIR/moon_8k.jpg"
    echo "  Done: $(du -h "$TEXTURE_DIR/moon_8k.jpg" | cut -f1)"
  fi

  if include_resolution 16k && should_process "$TEXTURE_DIR/moon_16k.jpg"; then
    echo "==> Fetching Moon 16K texture..."
    local src="$TEMP_DIR/moon_16k.tif"
    download "$MOON_16K_URL" "$src"
    echo "==> Creating moon_16k.jpg..."
    convert_jpeg "$src" "$TEXTURE_DIR/moon_16k.jpg"
    echo "  Done: $(du -h "$TEXTURE_DIR/moon_16k.jpg" | cut -f1)"
  fi
}

# Mars textures (USGS Viking MDIM 2.1 Colorized — 232m, 12GB GeoTIFF)
# 2k uses a small preview JPEG (1024px upscaled); 4k+ uses the full GeoTIFF.
process_mars() {
  # 2k: use preview JPEG (fast, no need for 12GB download)
  if include_resolution 2k && should_process "$TEXTURE_DIR/mars.jpg"; then
    echo "==> Fetching Mars 2K texture (MDIM 2.1 colorized preview)..."
    local preview="$TEMP_DIR/mars_preview.jpg"
    download "$MARS_PREVIEW_URL" "$preview"
    echo "==> Creating mars.jpg (2k)..."
    resize_jpeg "$preview" "$TEXTURE_DIR/mars.jpg" 2048 1024
    echo "  Done: $(du -h "$TEXTURE_DIR/mars.jpg" | cut -f1)"
  fi

  # 4k/8k/16k: full 12GB GeoTIFF
  local need_full=false
  for res in 4k 8k 16k; do
    if include_resolution "$res" && should_process "$TEXTURE_DIR/mars_${res}.jpg"; then
      need_full=true
    fi
  done

  if [[ "$need_full" == false ]]; then return; fi

  echo "==> Fetching Mars Viking MDIM 2.1 colorized mosaic (92160x46080, 12GB)..."
  echo "    This is a very large download and may take a long time."
  local src="$TEMP_DIR/mars_source.tif"
  download "$MARS_SOURCE_URL" "$src"

  if include_resolution 4k && should_process "$TEXTURE_DIR/mars_4k.jpg"; then
    echo "==> Creating mars_4k.jpg..."
    convert_resize_jpeg "$src" "$TEXTURE_DIR/mars_4k.jpg" 4096 2048
    echo "  Done: $(du -h "$TEXTURE_DIR/mars_4k.jpg" | cut -f1)"
  fi

  if include_resolution 8k && should_process "$TEXTURE_DIR/mars_8k.jpg"; then
    echo "==> Creating mars_8k.jpg..."
    convert_resize_jpeg "$src" "$TEXTURE_DIR/mars_8k.jpg" 8192 4096
    echo "  Done: $(du -h "$TEXTURE_DIR/mars_8k.jpg" | cut -f1)"
  fi

  if include_resolution 16k && should_process "$TEXTURE_DIR/mars_16k.jpg"; then
    echo "==> Creating mars_16k.jpg..."
    convert_resize_jpeg "$src" "$TEXTURE_DIR/mars_16k.jpg" 16384 8192
    echo "  Done: $(du -h "$TEXTURE_DIR/mars_16k.jpg" | cut -f1)"
  fi
}

# Sun texture (NASA SDO/STEREO — max ~4k)
process_sun() {
  local need_download=false
  if include_resolution 2k && should_process "$TEXTURE_DIR/sun.jpg"; then
    need_download=true
  fi
  if include_resolution 4k && should_process "$TEXTURE_DIR/sun_4k.jpg"; then
    need_download=true
  fi

  if [[ "$need_download" == false ]]; then return; fi

  echo "==> Fetching Sun SDO/STEREO EUV 304Å Carrington map (4104x2304)..."
  local src="$TEMP_DIR/sun_source.tif"
  download "$SUN_SOURCE_URL" "$src"

  if include_resolution 2k && should_process "$TEXTURE_DIR/sun.jpg"; then
    echo "==> Creating sun.jpg (2k)..."
    convert_resize_jpeg "$src" "$TEXTURE_DIR/sun.jpg" 2048 1024
    echo "  Done: $(du -h "$TEXTURE_DIR/sun.jpg" | cut -f1)"
  fi

  if include_resolution 4k && should_process "$TEXTURE_DIR/sun_4k.jpg"; then
    echo "==> Creating sun_4k.jpg (4k)..."
    convert_resize_jpeg "$src" "$TEXTURE_DIR/sun_4k.jpg" 4096 2048
    echo "  Done: $(du -h "$TEXTURE_DIR/sun_4k.jpg" | cut -f1)"
  fi
}

if include_body earth; then
  process_day
  process_night
fi
if include_body moon; then process_moon; fi
if include_body mars; then process_mars; fi
if include_body sun;  then process_sun;  fi

echo ""
echo "==> All done! Texture files in $TEXTURE_DIR:"
ls -lh "$TEXTURE_DIR"/*.jpg
