//! Sandboxed generated HTML is served from an origin without the application's IPC access.
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
const MAX_HTML_BYTES: usize = 2 * 1024 * 1024;
const MAX_PREVIEWS: usize = 32;
const PREVIEW_CSP: &str = "default-src 'none'; script-src 'unsafe-inline' https:; style-src 'unsafe-inline' https:; img-src https: data: blob:; font-src https: data:; connect-src https:; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; sandbox allow-scripts";
struct Preview {
    html: String,
    created: Instant,
}
fn previews() -> &'static Mutex<HashMap<String, Preview>> {
    static PREVIEWS: OnceLock<Mutex<HashMap<String, Preview>>> = OnceLock::new();
    PREVIEWS.get_or_init(Default::default)
}
#[tauri::command]
pub(crate) fn create_html_preview(html: String) -> Result<String, String> {
    if html.len() > MAX_HTML_BYTES {
        return Err("HTML preview exceeds 2 MiB".into());
    }
    let mut previews = previews().lock().map_err(|_| "HTML preview unavailable")?;
    previews.retain(|_, preview| preview.created.elapsed() < Duration::from_secs(3600));
    if previews.len() >= MAX_PREVIEWS {
        return Err("Too many HTML previews are open".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    previews.insert(
        id.clone(),
        Preview {
            html,
            created: Instant::now(),
        },
    );
    Ok(id)
}
#[tauri::command]
pub(crate) fn release_html_preview(id: String) {
    if let Ok(mut previews) = previews().lock() {
        previews.remove(&id);
    }
}
pub(crate) fn serve(request: tauri::http::Request<Vec<u8>>) -> tauri::http::Response<Vec<u8>> {
    let id = request.uri().path().trim_start_matches('/');
    let body = if request.method() == tauri::http::Method::GET {
        previews()
            .lock()
            .ok()
            .and_then(|previews| previews.get(id).map(|preview| preview.html.clone()))
    } else {
        None
    };
    let status = if body.is_some() { 200 } else { 404 };
    tauri::http::Response::builder()
        .status(status)
        .header("Content-Type", "text/html; charset=utf-8")
        .header("Content-Security-Policy", PREVIEW_CSP)
        .header("Cache-Control", "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .body(body.unwrap_or_default().into_bytes())
        .expect("static preview response headers")
}
#[cfg(test)]
mod tests {
    #[test]
    fn preview_serves_scripts_only_from_an_isolated_revocable_origin() {
        let html = "<script>document.body.textContent='interactive'</script>";
        let id = super::create_html_preview(html.into()).unwrap();
        let request = || {
            tauri::http::Request::builder()
                .uri(format!("kivio-preview://localhost/{id}"))
                .body(Vec::new())
                .unwrap()
        };
        let response = super::serve(request());
        assert_eq!(response.status(), 200);
        assert_eq!(response.body(), html.as_bytes());
        let policy = response.headers()["Content-Security-Policy"]
            .to_str()
            .unwrap();
        assert!(policy.contains("sandbox allow-scripts"));
        assert!(policy.contains("connect-src https:;"));
        assert!(!policy.contains("ipc:"));
        assert!(!policy.contains("allow-same-origin"));
        super::release_html_preview(id.clone());
        assert_eq!(super::serve(request()).status(), 404);
    }
}
