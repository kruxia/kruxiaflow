//! Built-in cost dashboard: self-contained static pages embedded in the
//! binary and mounted under the API router.
//!
//! Design: kruxiaflow-internal/docs/features/2026-07-18-dashboard.md — no Node
//! build, no external assets, no state. The page itself is public; the cost
//! endpoints it calls enforce API auth (credential-free under --insecure-dev).

use axum::Router;
use axum::response::Html;
use axum::routing::get;

const DASHBOARD_HTML: &str = include_str!("../assets/dashboard.html");

/// Dashboard routes, mountable into any router regardless of its state type.
pub fn router<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new()
        .route("/dashboard", get(|| async { Html(DASHBOARD_HTML) }))
        .route("/dashboard/", get(|| async { Html(DASHBOARD_HTML) }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_page_is_complete() {
        assert!(DASHBOARD_HTML.starts_with("<!DOCTYPE html>"));
        assert!(DASHBOARD_HTML.contains("</html>"));
        // Self-contained: no external fetches beyond the same-origin API
        assert!(!DASHBOARD_HTML.contains("https://cdn."));
        assert!(!DASHBOARD_HTML.contains("<script src"));
        assert!(!DASHBOARD_HTML.contains("<link rel=\"stylesheet\" href"));
    }

    #[test]
    fn router_builds() {
        let _router: Router<()> = router();
    }
}
