use axum::body::Body;
use axum::http::{HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::{Embed, EmbeddedFile};

#[derive(Embed)]
#[folder = "viewer-dist/"]
struct ViewerAssets;

pub(crate) fn get_asset(path: &str) -> Option<EmbeddedFile> {
    <ViewerAssets as Embed>::get(path)
}

pub async fn spa_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');

    // Try exact path match first
    if !path.is_empty()
        && let Some(file) = get_asset(path)
    {
        return serve_file(path, file);
    }

    // SPA fallback: serve index.html
    serve_index()
}

fn serve_file(path: &str, file: EmbeddedFile) -> Response {
    let mime = file.metadata.mimetype();

    let mut resp = (StatusCode::OK, Body::from(file.data.into_owned())).into_response();
    let content_type =
        HeaderValue::from_str(mime).unwrap_or(HeaderValue::from_static("application/octet-stream"));
    resp.headers_mut()
        .insert(header::CONTENT_TYPE, content_type);

    // Hashed asset filenames are immutable; index.html is not
    let cache = if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));

    resp
}

fn serve_index() -> Response {
    match get_asset("index.html") {
        Some(file) => {
            let mut resp = (StatusCode::OK, Body::from(file.data.into_owned())).into_response();
            resp.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            );
            resp.headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
            resp
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
