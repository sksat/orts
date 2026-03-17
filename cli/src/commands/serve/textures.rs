//! Texture serving: embedded 2K textures + on-demand high-resolution download.
//!
//! 2K textures for all bodies are compiled into the binary via `include_bytes!`.
//! Higher resolutions are downloaded from NASA/USGS in the background on server
//! start, converted/resized, and cached as JPEG in a tmpfs directory.

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path as AxumPath, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use image::ImageReader;
use image::imageops::FilterType;

// ---------------------------------------------------------------------------
// Embedded 2K textures (NASA/USGS public domain)
// ---------------------------------------------------------------------------

struct EmbeddedTexture {
    filename: &'static str,
    data: &'static [u8],
}

const EMBEDDED: &[EmbeddedTexture] = &[
    EmbeddedTexture {
        filename: "earth_2k.jpg",
        data: include_bytes!("../../../../viewer/public/textures/earth_2k.jpg"),
    },
    EmbeddedTexture {
        filename: "earth_night_2k.jpg",
        data: include_bytes!("../../../../viewer/public/textures/earth_night_2k.jpg"),
    },
    EmbeddedTexture {
        filename: "moon.jpg",
        data: include_bytes!("../../../../viewer/public/textures/moon.jpg"),
    },
    EmbeddedTexture {
        filename: "mars.jpg",
        data: include_bytes!("../../../../viewer/public/textures/mars.jpg"),
    },
    EmbeddedTexture {
        filename: "sun.jpg",
        data: include_bytes!("../../../../viewer/public/textures/sun.jpg"),
    },
];

// ---------------------------------------------------------------------------
// Download sources for high-resolution textures
// ---------------------------------------------------------------------------

/// A texture that needs to be downloaded and possibly resized.
struct DownloadTask {
    filename: &'static str,
    url: &'static str,
    /// Target width×height. `None` means convert to JPEG at original resolution.
    resize: Option<(u32, u32)>,
}

/// A group of textures sharing the same source download.
struct DownloadGroup {
    /// Human-readable description for logging.
    label: &'static str,
    url: &'static str,
    /// Tasks to produce from this single download.
    tasks: &'static [GroupTask],
}

struct GroupTask {
    filename: &'static str,
    resize: Option<(u32, u32)>,
}

// Earth day textures: single 21600×10800 source → 4k/8k/16k
static EARTH_DAY: DownloadGroup = DownloadGroup {
    label: "Blue Marble day (21600×10800)",
    url: "https://eoimages.gsfc.nasa.gov/images/imagerecords/73000/73909/world.topo.bathy.200412.3x21600x10800.jpg",
    tasks: &[
        GroupTask {
            filename: "earth_4k.jpg",
            resize: Some((4096, 2048)),
        },
        GroupTask {
            filename: "earth_8k.jpg",
            resize: Some((8192, 4096)),
        },
        GroupTask {
            filename: "earth_16k.jpg",
            resize: Some((16384, 8192)),
        },
    ],
};

// Earth night: low-res source for 4k
static EARTH_NIGHT_LOW: DownloadTask = DownloadTask {
    filename: "earth_night_4k.jpg",
    url: "https://assets.science.nasa.gov/content/dam/science/esd/eo/images/imagerecords/144000/144898/BlackMarble_2016_01deg.jpg",
    resize: Some((4096, 2048)),
};

// Earth night: high-res source for 8k/16k
static EARTH_NIGHT_HIGH: DownloadGroup = DownloadGroup {
    label: "Black Marble night (13500×6750)",
    url: "https://assets.science.nasa.gov/content/dam/science/esd/eo/images/imagerecords/144000/144898/BlackMarble_2016_3km.jpg",
    tasks: &[
        GroupTask {
            filename: "earth_night_8k.jpg",
            resize: Some((8192, 4096)),
        },
        GroupTask {
            filename: "earth_night_16k.jpg",
            resize: Some((16384, 8192)),
        },
    ],
};

// Moon: NASA provides pre-rendered TIFFs per resolution → just convert to JPEG
static MOON_DOWNLOADS: &[DownloadTask] = &[
    DownloadTask {
        filename: "moon_4k.jpg",
        url: "https://svs.gsfc.nasa.gov/vis/a000000/a004700/a004720/lroc_color_poles_4k.tif",
        resize: None,
    },
    DownloadTask {
        filename: "moon_8k.jpg",
        url: "https://svs.gsfc.nasa.gov/vis/a000000/a004700/a004720/lroc_color_poles_8k.tif",
        resize: None,
    },
    DownloadTask {
        filename: "moon_16k.jpg",
        url: "https://svs.gsfc.nasa.gov/vis/a000000/a004700/a004720/lroc_color_poles_16k.tif",
        resize: None,
    },
];

// Mars: 12GB GeoTIFF → 4k/8k/16k
static MARS: DownloadGroup = DownloadGroup {
    label: "Mars Viking MDIM 2.1 colorized (92160×46080, 12GB)",
    url: "https://planetarymaps.usgs.gov/mosaic/Mars_Viking_MDIM21_ClrMosaic_global_232m.tif",
    tasks: &[
        GroupTask {
            filename: "mars_4k.jpg",
            resize: Some((4096, 2048)),
        },
        GroupTask {
            filename: "mars_8k.jpg",
            resize: Some((8192, 4096)),
        },
        GroupTask {
            filename: "mars_16k.jpg",
            resize: Some((16384, 8192)),
        },
    ],
};

// Sun: ~4k TIFF → 4k JPEG
static SUN: DownloadTask = DownloadTask {
    filename: "sun_4k.jpg",
    url: "https://svs.gsfc.nasa.gov/vis/a030000/a030300/a030362/euvi_aia304_2012_carrington.tif",
    resize: Some((4096, 2048)),
};

// ---------------------------------------------------------------------------
// TextureCache
// ---------------------------------------------------------------------------

pub struct TextureCache {
    embedded: HashMap<&'static str, &'static [u8]>,
    cache_dir: PathBuf,
}

impl TextureCache {
    pub fn new() -> Self {
        let cache_dir = PathBuf::from("/tmp/orts/textures");
        let mut embedded = HashMap::new();
        for tex in EMBEDDED {
            embedded.insert(tex.filename, tex.data);
        }
        Self {
            embedded,
            cache_dir,
        }
    }

    /// Look up a texture by filename: first embedded, then cache directory.
    pub fn get(&self, filename: &str) -> Option<TextureData> {
        // 1. Embedded textures
        if let Some(&data) = self.embedded.get(filename) {
            return Some(TextureData::Embedded(data));
        }
        // 2. Cached on disk
        let path = self.cache_dir.join(filename);
        if path.is_file() {
            return Some(TextureData::Cached(path));
        }
        None
    }
}

pub enum TextureData {
    Embedded(&'static [u8]),
    Cached(PathBuf),
}

// ---------------------------------------------------------------------------
// axum handler
// ---------------------------------------------------------------------------

pub async fn texture_handler(
    AxumPath(filename): AxumPath<String>,
    State(cache): State<Arc<TextureCache>>,
) -> Response {
    match cache.get(&filename) {
        Some(TextureData::Embedded(data)) => {
            let mut resp = (StatusCode::OK, Body::from(data)).into_response();
            resp.headers_mut()
                .insert(header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg"));
            resp.headers_mut().insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=86400, immutable"),
            );
            resp
        }
        Some(TextureData::Cached(path)) => match tokio::fs::read(&path).await {
            Ok(data) => {
                let mut resp = (StatusCode::OK, Body::from(data)).into_response();
                resp.headers_mut()
                    .insert(header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg"));
                resp.headers_mut().insert(
                    header::CACHE_CONTROL,
                    HeaderValue::from_static("public, max-age=86400"),
                );
                resp
            }
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        },
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

// ---------------------------------------------------------------------------
// Background downloader
// ---------------------------------------------------------------------------

pub fn spawn_background_downloads(cache: Arc<TextureCache>) {
    tokio::spawn(async move {
        let dir = &cache.cache_dir;
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("Failed to create texture cache dir {}: {e}", dir.display());
            return;
        }

        // Priority order: Earth → Moon → Sun → Mars (Mars is huge)
        download_group(dir, &EARTH_DAY).await;
        download_single(dir, &EARTH_NIGHT_LOW).await;
        download_group(dir, &EARTH_NIGHT_HIGH).await;

        for task in MOON_DOWNLOADS {
            download_single(dir, task).await;
        }

        download_single(dir, &SUN).await;

        download_group(dir, &MARS).await;

        eprintln!("Background texture downloads complete");
    });
}

async fn download_single(cache_dir: &Path, task: &DownloadTask) {
    let dest = cache_dir.join(task.filename);
    if dest.is_file() {
        return;
    }
    eprintln!("Downloading texture: {}...", task.filename);

    match fetch_and_process(task.url, task.resize).await {
        Ok(jpeg_data) => match std::fs::write(&dest, &jpeg_data) {
            Ok(()) => eprintln!(
                "  {} ready ({:.1} MB)",
                task.filename,
                jpeg_data.len() as f64 / 1_048_576.0
            ),
            Err(e) => eprintln!("  Failed to write {}: {e}", task.filename),
        },
        Err(e) => eprintln!("  Failed to process {}: {e}", task.filename),
    }
}

async fn download_group(cache_dir: &Path, group: &DownloadGroup) {
    // Check if all outputs already exist
    let needed: Vec<&GroupTask> = group
        .tasks
        .iter()
        .filter(|t| !cache_dir.join(t.filename).is_file())
        .collect();

    if needed.is_empty() {
        return;
    }

    eprintln!(
        "Downloading {} for {} texture(s)...",
        group.label,
        needed.len()
    );

    // Download once
    let data = match tokio::task::spawn_blocking({
        let url = group.url.to_string();
        move || download_bytes(&url)
    })
    .await
    {
        Ok(Ok(data)) => data,
        Ok(Err(e)) => {
            eprintln!("  Download failed: {e}");
            return;
        }
        Err(e) => {
            eprintln!("  Download task panicked: {e}");
            return;
        }
    };

    // Process each needed resolution
    for task in &needed {
        let dest = cache_dir.join(task.filename);
        match process_image_data(&data, task.resize) {
            Ok(jpeg_data) => match std::fs::write(&dest, &jpeg_data) {
                Ok(()) => eprintln!(
                    "  {} ready ({:.1} MB)",
                    task.filename,
                    jpeg_data.len() as f64 / 1_048_576.0
                ),
                Err(e) => eprintln!("  Failed to write {}: {e}", task.filename),
            },
            Err(e) => eprintln!("  Failed to process {}: {e}", task.filename),
        }
    }
}

/// Download + optionally resize, returning JPEG bytes.
async fn fetch_and_process(
    url: &str,
    resize: Option<(u32, u32)>,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let url = url.to_string();
    tokio::task::spawn_blocking(move || {
        let data = download_bytes(&url)?;
        process_image_data(&data, resize)
    })
    .await?
}

fn download_bytes(url: &str) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let resp = ureq::get(url).call()?;
    let len = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(10_000_000);
    let mut data = Vec::with_capacity(len);
    resp.into_body().into_reader().read_to_end(&mut data)?;
    Ok(data)
}

fn process_image_data(
    data: &[u8],
    resize: Option<(u32, u32)>,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let img = ImageReader::new(Cursor::new(data))
        .with_guessed_format()?
        .decode()?;

    let img = match resize {
        Some((w, h)) => img.resize_exact(w, h, FilterType::Lanczos3),
        None => img,
    };

    let mut jpeg_buf = Vec::new();
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_buf, 90);
    img.write_with_encoder(encoder)?;
    Ok(jpeg_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_textures_are_loaded() {
        let cache = TextureCache::new();
        assert!(cache.get("earth_2k.jpg").is_some());
        assert!(cache.get("earth_night_2k.jpg").is_some());
        assert!(cache.get("moon.jpg").is_some());
        assert!(cache.get("mars.jpg").is_some());
        assert!(cache.get("sun.jpg").is_some());
    }

    #[test]
    fn unknown_texture_returns_none() {
        let cache = TextureCache::new();
        assert!(cache.get("nonexistent.jpg").is_none());
    }

    #[test]
    fn embedded_data_is_valid_jpeg() {
        let cache = TextureCache::new();
        for name in &[
            "earth_2k.jpg",
            "earth_night_2k.jpg",
            "moon.jpg",
            "mars.jpg",
            "sun.jpg",
        ] {
            if let Some(TextureData::Embedded(data)) = cache.get(name) {
                // JPEG files start with FF D8 FF
                assert!(
                    data.len() > 3 && data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF,
                    "{name} is not a valid JPEG"
                );
            } else {
                panic!("{name} not found in embedded textures");
            }
        }
    }
}
