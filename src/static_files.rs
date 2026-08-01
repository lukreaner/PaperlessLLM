use axum::{
    body::Body,
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "web/dist/"]
struct DashboardAssets;

pub async fn serve(uri: Uri) -> Response {
    let requested = uri.path().trim_start_matches('/');
    let path = if requested.is_empty() {
        "index.html"
    } else {
        requested
    };

    if let Some(asset) = DashboardAssets::get(path) {
        return asset_response(path, asset.data.into_owned());
    }

    // Client-side routes such as /review and /settings are served by the SPA.
    if !path.contains('.')
        && let Some(index) = DashboardAssets::get("index.html")
    {
        return asset_response("index.html", index.data.into_owned());
    }

    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        "Dashboard assets are not embedded. Build web/ before compiling PaperlessLLM.",
    )
        .into_response()
}

fn asset_response(path: &str, bytes: Vec<u8>) -> Response {
    let content_type = match path.rsplit('.').next().unwrap_or_default() {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    };
    let cache_control = if path == "index.html" {
        "no-cache"
    } else {
        "public, max-age=31536000, immutable"
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, cache_control)
        .body(Body::from(bytes))
        .expect("valid embedded asset response")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn missing_asset_is_safe() {
        let response = serve(Uri::from_static("/definitely-missing.js")).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
