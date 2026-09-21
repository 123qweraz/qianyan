use std::path::Path;
use std::io::Cursor;
use axum::{
    extract::Multipart,
    response::IntoResponse,
    http::StatusCode,
    Json,
};
use regex::Regex;
use serde::Serialize;
use epub::doc::EpubDoc;
use base64::Engine;

use super::extract_file_bytes;

#[derive(Serialize)]
pub(crate) struct EpubChapter {
    label: String,
    text: String,
    html: String,
}

#[derive(Serialize)]
pub(crate) struct ParseEpubResponse {
    title: String,
    chapters: Vec<EpubChapter>,
}

pub(crate) async fn parse_epub_handler(
    multipart: Multipart,
) -> impl IntoResponse {
    let bytes = match extract_file_bytes(multipart).await {
        Ok(b) => b,
        Err((code, msg)) => return (code, msg).into_response(),
    };

    let result = tokio::task::spawn_blocking(move || {
        parse_epub_inner(bytes)
    }).await;

    match result {
        Ok(Ok(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(Err(msg)) => (StatusCode::BAD_REQUEST, msg).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "EPUB 解析线程崩溃".to_string()).into_response(),
    }
}

fn parse_epub_inner(bytes: Vec<u8>) -> Result<ParseEpubResponse, String> {
    let cursor = Cursor::new(bytes);
    let mut doc = EpubDoc::from_reader(cursor)
        .map_err(|e| format!("EPUB 解析失败: {e}"))?;

    let title = doc.get_title().unwrap_or_else(|| "未知书名".into());
    let num = doc.get_num_chapters();
    if num == 0 {
        return Err("EPUB 中没有内容".into());
    }

    let re_script = Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap();
    let re_style = Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap();
    let re_xml = Regex::new(r"(?is)<\?xml[^>]*\?>").unwrap();
    let re_doctype = Regex::new(r"(?is)<!DOCTYPE[^>]*>").unwrap();
    let re_surround = Regex::new(r"(?is)</?(?:html|head|body)[^>]*>").unwrap();

    let mut chapters = Vec::with_capacity(num);
    for i in 0..num {
        if !doc.set_current_chapter(i) {
            continue;
        }
        let (raw_html, _) = match doc.get_current_str() {
            Some(v) => v,
            None => continue,
        };

        let cleaned = re_xml.replace_all(&raw_html, "");
        let cleaned = re_doctype.replace_all(&cleaned, "");
        let cleaned = re_script.replace_all(&cleaned, "");
        let cleaned = re_style.replace_all(&cleaned, "");
        let cleaned = re_surround.replace_all(&cleaned, "");
        let html = embed_images(&mut doc, &cleaned);

        let html = html.trim().to_string();
        if html.is_empty() {
            continue;
        }

        let text = strip_html_to_text(&html);
        let label = extract_chapter_label(&text, i);
        chapters.push(EpubChapter { label, text, html });
    }

    if chapters.is_empty() {
        return Err("未能从 EPUB 中提取到文本内容".into());
    }

    Ok(ParseEpubResponse { title, chapters })
}

fn embed_images(doc: &mut EpubDoc<Cursor<Vec<u8>>>, html: &str) -> String {
    let re_img = Regex::new(r#"(?i)<img\s+[^>]*src\s*=\s*"([^"]+)"[^>]*>"#).unwrap();
    let ch_path = doc.get_current_path().unwrap_or_else(|| Path::new("").to_path_buf());

    re_img.replace_all(html, |caps: &regex::Captures| {
        let src = &caps[1];
        let img_path = ch_path.parent().unwrap_or(Path::new("")).join(src);
        match doc.get_resource_by_path(&img_path) {
            Some(data) => {
                let mime = doc.get_resource_mime_by_path(&img_path)
                    .unwrap_or_else(|| "image/png".to_string());
                let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
                format!(r#"<img src="data:{};base64,{}" />"#, mime, b64)
            }
            None => caps[0].to_string(),
        }
    }).to_string()
}

fn strip_html_to_text(html: &str) -> String {
    let re_br = Regex::new(r"(?i)<br\s*/?>").unwrap();
    let re_tag = Regex::new(r"<[^>]*>").unwrap();
    let re_entity = Regex::new(r"&(?:amp|lt|gt|nbsp|quot);").unwrap();

    let cleaned = re_br.replace_all(html, "\n");
    let cleaned = re_tag.replace_all(&cleaned, "");
    let cleaned = re_entity.replace_all(&cleaned, |caps: &regex::Captures| -> String {
        match &caps[0] {
            "&amp;" => "&",
            "&lt;" => "<",
            "&gt;" => ">",
            "&nbsp;" => " ",
            "&quot;" => "\"",
            _ => &caps[0],
        }.to_string()
    });
    cleaned.trim().to_string()
}

fn extract_chapter_label(text: &str, index: usize) -> String {
    let heading_re = Regex::new(r"(?m)^(.{1,80})$").unwrap();
    for cap in heading_re.captures_iter(text) {
        let line = cap[1].trim();
        if line.chars().count() > 3 && line.chars().count() <= 80 {
            return line.to_string();
        }
    }
    format!("第{}章", index + 1)
}