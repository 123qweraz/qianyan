use axum::{
    response::{IntoResponse, Html},
    http::{StatusCode, Uri, HeaderName, header},
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../static/"]
pub struct Assets;

/// Config server index handler: 控制中心 — settings only, no sidebar.
pub async fn config_index_handler() -> impl IntoResponse {
    match Assets::get("index.html") {
        Some(content) => {
            let html = String::from_utf8_lossy(&content.data);
            let modified = html
                .replace("<script src=\"/static/js/sidebar.js\"></script>", "")
                .replace("</body>", "<script>document.querySelectorAll('.feature-only').forEach(function(e){e.remove()});var h=document.querySelector('h1');if(h)h.textContent='千言输入法 · 控制中心';document.title='千言输入法 · 控制中心'</script></body>");
            Html(modified).into_response()
        }
        None => (StatusCode::NOT_FOUND, "Not Found").into_response(),
    }
}

/// Feature server index handler: 功能中心 — features only, with sidebar.
pub async fn feature_index_handler() -> impl IntoResponse {
    match Assets::get("index.html") {
        Some(content) => {
            let html = String::from_utf8_lossy(&content.data);
            let modified = html.replace("</body>",
                "<script>document.querySelectorAll('.config-only').forEach(function(e){e.remove()});var s=document.querySelector('.qy-sidebar-nav a[href=\"#section-settings\"]');if(s)s.remove();var h=document.querySelector('h1');if(h)h.textContent='千言输入法 · 功能中心';document.title='千言输入法 · 功能中心'</script></body>");
            Html(modified).into_response()
        }
        None => (StatusCode::NOT_FOUND, "Not Found").into_response(),
    }
}

pub async fn static_handler(uri: Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches("/static/").trim_start_matches("/");

    let dev_root = find_static_root();
    if let Some(ref dev_root) = dev_root {
        if let Some(safe_path) = safe_join(dev_root, path) {
            if let Ok(content) = std::fs::read(&safe_path) {
                let mime = mime_guess::from_path(&safe_path).first_or_octet_stream();
                let headers = [
                    (header::CONTENT_TYPE, mime.as_ref()),
                    (HeaderName::from_static("cache-control"), "no-cache"),
                ];
                return (headers, content).into_response();
            }
        }
    }

    match Assets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            let headers = [
                (header::CONTENT_TYPE, mime.as_ref()),
                (HeaderName::from_static("cache-control"), "no-cache"),
            ];
            (headers, content.data).into_response()
        }
        None => (StatusCode::NOT_FOUND, "Not Found").into_response(),
    }
}

pub(crate) fn find_static_root() -> Option<std::path::PathBuf> {
    let cwd_static = std::path::PathBuf::from("static");
    if cwd_static.is_dir() {
        return Some(cwd_static);
    }

    if let Some(exe_dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())) {
        let exe_static = exe_dir.join("static");
        if exe_static.is_dir() {
            return Some(exe_static);
        }
    }

    if let Ok(mut cwd) = std::env::current_dir() {
        for _ in 0..3 {
            let check = cwd.join("static");
            if check.is_dir() {
                return Some(check);
            }
            cwd.pop();
        }
    }
    None
}

pub(crate) fn find_dicts_root() -> std::path::PathBuf {
    let base_path = std::path::PathBuf::from("dicts");
    if base_path.exists() {
        return base_path;
    }
    if let Ok(mut exe_path) = std::env::current_exe() {
        exe_path.pop();
        for _ in 0..3 {
            let p = exe_path.join("dicts");
            if p.exists() {
                return p;
            }
            exe_path.pop();
        }
    }
    std::path::PathBuf::from("dicts")
}

pub(crate) fn valid_profile_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

pub(crate) fn safe_join(base: &std::path::Path, user_path: &str) -> Option<std::path::PathBuf> {
    use std::path::Component;
    let p = std::path::Path::new(user_path);
    let base = base.canonicalize().ok()?;

    if p.is_absolute() {
        let canonical = p.canonicalize().ok()?;
        return if canonical.starts_with(&base) { Some(canonical) } else { None };
    }
    for c in p.components() {
        if matches!(c, Component::ParentDir) {
            return None;
        }
    }
    let joined = base.join(p);
    if joined.exists() {
        let canonical = joined.canonicalize().ok()?;
        if canonical.starts_with(&base) {
            return Some(canonical);
        }
        return None;
    }
    let parent = joined.parent()?;
    let parent_canonical = parent.canonicalize().ok()?;
    if parent_canonical.starts_with(&base) {
        Some(joined)
    } else {
        None
    }
}

pub async fn dicts_handler(uri: Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches("/dicts/").trim_start_matches("/");
    
    let base_path = find_dicts_root();
    if let Some(safe_path) = safe_join(&base_path, path) {
        if let Ok(content) = std::fs::read(&safe_path) {
            let mime = mime_guess::from_path(&safe_path).first_or_octet_stream();
            return ([(axum::http::header::CONTENT_TYPE, mime.as_ref())], content).into_response();
        }
    }
    eprintln!("[Web] Dictionary file not found: {} in {:?}", path, base_path);
    (StatusCode::NOT_FOUND, "Dictionary Not Found").into_response()
}
