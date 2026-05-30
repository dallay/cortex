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
        .route("/*path", get(serve_assets))
        .fallback(serve_index_fallback)
}

async fn serve_index() -> impl IntoResponse {
    serve_dashboard_asset("index.html")
}

async fn serve_assets(Path(path): Path<String>) -> impl IntoResponse {
    serve_dashboard_asset(&path)
}

async fn serve_index_fallback() -> impl IntoResponse {
    serve_dashboard_asset("index.html")
}

fn serve_dashboard_asset(path: &str) -> impl IntoResponse {
    match DashboardAssets::get(path) {
        Some(asset) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
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