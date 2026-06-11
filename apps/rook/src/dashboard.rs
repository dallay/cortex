// dashboard — serve embedded Vue SPA static assets
use axum::{
    extract::Path,
    http::{header, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};
use bytes::Bytes;
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "dashboard/dist"]
struct DashboardAssets;

pub fn dashboard_routes() -> Router {
    Router::new()
        .route("/", get(serve_index))
        .route("/{*path}", get(serve_assets))
}

async fn serve_index() -> impl IntoResponse {
    serve_dashboard_asset("index.html")
}

async fn serve_assets(Path(path): Path<String>) -> impl IntoResponse {
    serve_dashboard_asset(&path)
}

/// Root path handler — serves the dashboard SPA at the root URL (/)
/// This allows accessing the dashboard at http://host/ directly instead of /dashboard/
pub async fn root_handler() -> impl IntoResponse {
    serve_dashboard_asset("index.html")
}

pub fn serve_dashboard_asset(path: &str) -> impl IntoResponse {
    // Strip leading slash — RustEmbed stores paths without leading slash
    let normalized = path.strip_prefix('/').unwrap_or(path);
    match DashboardAssets::get(normalized) {
        Some(asset) => {
            let mime = mime_guess::from_path(normalized).first_or_octet_stream();
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, mime.as_ref())],
                Bytes::copy_from_slice(&asset.data),
            )
                .into_response()
        }
        None => {
            // Fallback to index.html for SPA routing (Vue Router)
            match DashboardAssets::get("index.html") {
                Some(asset) => (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                    Bytes::copy_from_slice(&asset.data),
                )
                    .into_response(),
                None => (StatusCode::NOT_FOUND, "Not found").into_response(),
            }
        }
    }
}
