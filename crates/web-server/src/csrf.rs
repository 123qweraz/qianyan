use axum::{
    extract::Request,
    http::{header, HeaderMap, Method, StatusCode},
    middleware::Next,
    response::Response,
};

/// 判定请求是否被允许：
/// - GET/HEAD 等安全方法不校验（读操作，跨站风险低）
/// - 无 Origin/Referer（curl、本机进程等非浏览器调用）放行
/// - Origin/Referer 为主机名属于回环地址（127.0.0.1/localhost/::1）放行
/// - 其余（外部网站触发的跨站 POST）拒绝，防 CSRF
pub(crate) fn is_allowed(method: &Method, headers: &HeaderMap) -> bool {
    if method == Method::GET || method == Method::HEAD {
        return true;
    }
    let source = headers
        .get(header::ORIGIN)
        .or_else(|| headers.get(header::REFERER));
    match source.and_then(|v| v.to_str().ok()) {
        None => true,
        Some(s) => host_is_loopback(s),
    }
}

/// 从 Origin/Referer 中提取主机名并判断是否回环地址。
fn host_is_loopback(source: &str) -> bool {
    // 兼容 chrome://... 等无 "://" 的形式，直接视为非 http 源路径
    let after_scheme = match source.split_once("://") {
        Some((_, rest)) => rest,
        None => source,
    };
    let host = if let Some(rest) = after_scheme.strip_prefix('[') {
        // IPv6 括号形式：取到 ']' 为止
        match rest.split_once(']') {
            Some((h, _)) => format!("[{}]", h),
            None => return false,
        }
    } else {
        // IPv4/主机名：截到第一个 ':'（端口）或 '/'（路径）之前
        after_scheme
            .split([':', '/'])
            .next()
            .unwrap_or(after_scheme)
            .to_string()
    };
    matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1" | "[::1]" | "0.0.0.0")
}

/// CSRF 防护中间件：对状态变更请求校验 Origin/Referer 为回环地址。
pub(crate) async fn csrf_layer(request: Request, next: Next) -> Result<Response, StatusCode> {
    if !is_allowed(request.method(), request.headers()) {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(next.run(request).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(origin: Option<&str>, referer: Option<&str>) -> HeaderMap {
        let mut h = HeaderMap::new();
        if let Some(o) = origin {
            h.insert(header::ORIGIN, HeaderValue::from_str(o).unwrap());
        }
        if let Some(r) = referer {
            h.insert(header::REFERER, HeaderValue::from_str(r).unwrap());
        }
        h
    }

    #[test]
    fn get_always_allowed() {
        assert!(is_allowed(&Method::GET, &headers(Some("https://evil.com"), None)));
    }

    #[test]
    fn post_without_origin_allowed() {
        assert!(is_allowed(&Method::POST, &headers(None, None)));
    }

    #[test]
    fn post_from_loopback_allowed() {
        for src in [
            "http://127.0.0.1:18765",
            "http://localhost:18766",
            "http://127.0.0.1:9999/config",
            "http://[::1]:18765",
        ] {
            assert!(is_allowed(&Method::POST, &headers(Some(src), None)), "{src}");
        }
    }

    #[test]
    fn post_from_external_origin_blocked() {
        assert!(!is_allowed(
            &Method::POST,
            &headers(Some("https://evil.com"), None)
        ));
        assert!(!is_allowed(
            &Method::POST,
            &headers(None, Some("https://evil.com/phishing"))
        ));
        assert!(!is_allowed(&Method::POST, &headers(Some("null"), None)));
        assert!(!is_allowed(&Method::POST, &headers(Some("http://10.0.0.1:80"), None)));
    }

    #[test]
    fn origin_takes_precedence_over_referer() {
        // Origin 可信但 Referer 恶意 → 以 Origin 为准放行（浏览器同时带两者时 Origin 更准确）
        assert!(is_allowed(
            &Method::POST,
            &headers(Some("http://127.0.0.1:18765"), Some("https://evil.com"))
        ));
    }
}