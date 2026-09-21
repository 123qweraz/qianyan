use super::*;

#[derive(serde::Deserialize)]
pub(crate) struct SendKeyRequest {
    key: String,
    action: Option<String>,
}

pub(crate) async fn send_key_handler(
    State(state): State<WebState>,
    Json(req): Json<SendKeyRequest>
) -> StatusCode {
    // CSRF 校验已由全局 csrf 中间件统一处理（校验 Origin/Referer 为回环地址）

    let key = req.key.to_lowercase();
    let action = req.action.unwrap_or_else(|| "tap".to_string());

    let tray_tx = state.2.clone();
    let _ = tray_tx.send(TrayEvent::SendKey { key, action });

    StatusCode::OK
}

#[derive(serde::Deserialize)]
pub(crate) struct PinyinConvertRequest {
    text: String,
}

#[derive(Serialize)]
pub(crate) struct PinyinConvertResponse {
    segmented: String,
    pinyin: String,
}

pub(crate) type WordMap = HashMap<String, (String, u32)>;

static CHINESE_WORD_MAP: OnceLock<WordMap> = OnceLock::new();

fn load_chinese_word_map() -> WordMap {
    let root = qianyan_ime_core::utils::find_project_root();
    let data_dir = root.join("data");
    let index_path = data_dir.join("chinese/trie.index");
    let data_path = data_dir.join("chinese/trie.data");

    let trie = match Trie::load(&index_path, &data_path, false) {
        Ok(t) => t,
        Err(e) => {
            log::warn!("[pinyin] Failed to load chinese trie: {}", e);
            return HashMap::new();
        }
    };

    let mut map = WordMap::new();
    let mut stream = trie.index.stream();
    while let Some((pinyin_bytes, offset)) = stream.next() {
        let pinyin = String::from_utf8_lossy(pinyin_bytes).to_string();
        trie.read_block(offset as usize, |tr| {
            let word = tr.word.to_string();
            map.entry(word).or_insert_with(|| (pinyin.clone(), tr.weight));
        });
    }
    map
}

pub(crate) fn get_chinese_word_map() -> &'static WordMap {
    CHINESE_WORD_MAP.get_or_init(load_chinese_word_map)
}

fn is_word_in_map(word: &str, map: &WordMap) -> bool {
    map.contains_key(word)
}

fn word_weight(word: &str, map: &WordMap) -> u32 {
    map.get(word).map_or(0, |e| e.1)
}

fn word_pinyin(word: &str, map: &WordMap) -> String {
    map.get(word).map_or_else(|| word.to_string(), |e| e.0.clone())
}

fn max_word_len(map: &WordMap) -> usize {
    map.keys().map(|k| k.chars().count()).max().unwrap_or(6).min(8)
}

fn segment_forward(chars: &[char], map: &WordMap, max_len: usize) -> Vec<String> {
    let mut words = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !is_chinese(chars[i]) {
            words.push(chars[i].to_string());
            i += 1;
            continue;
        }
        let lookahead = max_len.min(chars.len() - i);
        let mut found = false;
        for len in (1..=lookahead).rev() {
            let word: String = chars[i..i + len].iter().collect();
            if is_word_in_map(&word, map) {
                words.push(word);
                i += len;
                found = true;
                break;
            }
        }
        if !found {
            words.push(chars[i].to_string());
            i += 1;
        }
    }
    words
}

fn segment_backward(chars: &[char], map: &WordMap, max_len: usize) -> Vec<String> {
    let mut words = Vec::new();
    let mut i = chars.len();
    while i > 0 {
        if !is_chinese(chars[i - 1]) {
            words.push(chars[i - 1].to_string());
            i -= 1;
            continue;
        }
        let lookahead = max_len.min(i);
        let mut found = false;
        for len in (1..=lookahead).rev() {
            let word: String = chars[i - len..i].iter().collect();
            if is_word_in_map(&word, map) {
                words.push(word);
                i -= len;
                found = true;
                break;
            }
        }
        if !found {
            words.push(chars[i - 1].to_string());
            i -= 1;
        }
    }
    words.reverse();
    words
}

fn single_char_count(words: &[String]) -> usize {
    words.iter().filter(|w| w.chars().count() == 1).count()
}

fn total_weight(words: &[String], map: &WordMap) -> u64 {
    words.iter().map(|w| word_weight(w, map) as u64).sum()
}

fn segment_and_convert(
    text: &str,
    map: &WordMap,
) -> (String, String) {
    let chars: Vec<char> = text.chars().collect();
    let max_len = max_word_len(map);

    let forward = segment_forward(&chars, map, max_len);
    let backward = segment_backward(&chars, map, max_len);

    let words = if forward == backward {
        &forward
    } else {
        let f1 = single_char_count(&forward);
        let f2 = single_char_count(&backward);
        if f1 != f2 {
            if f1 < f2 { &forward } else { &backward }
        } else {
            let fw = total_weight(&forward, map);
            let bw = total_weight(&backward, map);
            if fw >= bw { &forward } else { &backward }
        }
    };

    let mut segmented = Vec::new();
    let mut pinyin = Vec::new();
    for word in words {
        segmented.push(word.clone());
        pinyin.push(word_pinyin(word, map));
    }

    (segmented.join(" "), pinyin.join(" "))
}

pub(crate) async fn pinyin_convert_handler(
    State(_): State<WebState>,
    Json(req): Json<PinyinConvertRequest>,
) -> Json<PinyinConvertResponse> {
    let map = get_chinese_word_map();
    if map.is_empty() {
        return Json(PinyinConvertResponse {
            segmented: req.text.clone(),
            pinyin: req.text,
        });
    }
    let (segmented, pinyin) = segment_and_convert(&req.text, map);
    Json(PinyinConvertResponse { segmented, pinyin })
}

fn is_chinese(c: char) -> bool {
    ('\u{4e00}'..='\u{9fa5}').contains(&c) || ('\u{3400}'..='\u{4dbf}').contains(&c)
}

// ===== Simplified ⇄ Traditional conversion =====

pub(crate) type S2TMap = HashMap<String, String>;

pub(crate) struct S2TData {
    s2t: S2TMap,
    t2s: S2TMap,
    s2t_max_len: usize,
    t2s_max_len: usize,
}

static S2T_DATA: OnceLock<S2TData> = OnceLock::new();

fn load_s2t_t2s_maps() -> (S2TMap, S2TMap, usize, usize) {
    let root = qianyan_ime_core::utils::find_project_root();
    let data_dir = root.join("data");
    let index_path = data_dir.join("chinese/trie.index");
    let data_path = data_dir.join("chinese/trie.data");

    let trie = match Trie::load(&index_path, &data_path, false) {
        Ok(t) => t,
        Err(e) => {
            log::warn!("[s2t] Failed to load chinese trie: {}", e);
            return (HashMap::new(), HashMap::new(), 0, 0);
        }
    };

    let mut s2t: S2TMap = HashMap::new();
    let mut t2s: S2TMap = HashMap::new();
    let mut s2t_max = 0usize;
    let mut t2s_max = 0usize;

    let mut stream = trie.index.stream();
    while let Some((_pinyin_bytes, offset)) = stream.next() {
        trie.read_block(offset as usize, |tr| {
            let word = tr.word.to_string();
            let trad = tr.trad.to_string();
            if !trad.is_empty() && word != trad {
                let wc = word.chars().count();
                if wc > s2t_max { s2t_max = wc; }
                // For s2t: prefer longer match, then higher weight
                s2t.entry(word.clone())
                    .and_modify(|existing| {
                        let existing_len = existing.chars().count();
                        if wc > existing_len {
                            *existing = trad.clone();
                        }
                    })
                    .or_insert_with(|| trad.clone());

                let tc = trad.chars().count();
                if tc > t2s_max { t2s_max = tc; }
                t2s.entry(trad)
                    .and_modify(|existing| {
                        let existing_len = existing.chars().count();
                        if tc > existing_len {
                            *existing = word.clone();
                        }
                    })
                    .or_insert_with(|| word.clone());
            }
        });
    }

    (s2t, t2s, s2t_max, t2s_max)
}

fn get_s2t_data() -> &'static S2TData {
    S2T_DATA.get_or_init(|| {
        let (s2t, t2s, s2t_max, t2s_max) = load_s2t_t2s_maps();
        S2TData { s2t, t2s, s2t_max_len: s2t_max, t2s_max_len: t2s_max }
    })
}
fn get_s2t_map() -> &'static S2TMap { &get_s2t_data().s2t }
fn get_t2s_map() -> &'static S2TMap { &get_s2t_data().t2s }
fn get_s2t_max_len() -> usize { get_s2t_data().s2t_max_len }
fn get_t2s_max_len() -> usize { get_s2t_data().t2s_max_len }
fn ensure_s2t_loaded() { let _ = get_s2t_data(); }

fn convert_longest_match(text: &str, map: &S2TMap, max_len: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut result = String::with_capacity(text.len() * 2);
    let mut i = 0;
    while i < chars.len() {
        let mut best_len = 0usize;
        let mut best_word = None;
        let max_check = max_len.min(chars.len() - i);
        for len in (1..=max_check).rev() {
            let slice: String = chars[i..i + len].iter().collect();
            if let Some(converted) = map.get(&slice) {
                best_len = len;
                best_word = Some(converted.clone());
                break;
            }
        }
        if best_len > 0 {
            result.push_str(&best_word.expect("best_word must be Some when best_len > 0"));
            i += best_len;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

#[derive(Deserialize)]
pub(crate) struct ConvertRequest {
    text: String,
    #[serde(default = "default_convert_mode")]
    mode: String,
}

fn default_convert_mode() -> String { "pinyin".into() }

#[derive(Serialize)]
pub(crate) struct ConvertResponse {
    result: String,
    segmented: Option<String>,
    pinyin: Option<String>,
}

fn cn_punct_to_en(c: char) -> Option<char> {
    match c {
        '，' => Some(','),
        '。' => Some('.'),
        '！' => Some('!'),
        '？' => Some('?'),
        '；' => Some(';'),
        '：' => Some(':'),
        '\u{201c}' | '\u{201d}' => Some('"'),
        '\u{2018}' | '\u{2019}' => Some('\''),
        '（' => Some('('),
        '）' => Some(')'),
        '【' => Some('['),
        '】' => Some(']'),
        '《' => Some('<'),
        '》' => Some('>'),
        '、' => Some(','),
        '～' => Some('~'),
        '…' => Some('.'),
        '—' => Some('-'),
        _ => None,
    }
}

fn segment_and_convert_pinyin(text: &str, map: &WordMap) -> (String, String) {
    let chars: Vec<char> = text.chars().collect();
    let max_len = max_word_len(map);
    let mut segmented = Vec::new();
    let mut pinyin_parts = Vec::new();
    let mut i = 0;

    while i < chars.len() {
        if !is_chinese(chars[i]) {
            // Punctuation / non-Chinese: keep in segmented, convert for pinyin
            segmented.push(chars[i].to_string());
            if let Some(en) = cn_punct_to_en(chars[i]) {
                pinyin_parts.push(en.to_string());
            } else if chars[i].is_whitespace() {
                pinyin_parts.push(" ".to_string());
            } else {
                pinyin_parts.push(chars[i].to_string());
            }
            i += 1;
            continue;
        }

        let mut best_len = 0usize;
        let max_check = max_len.min(chars.len() - i);
        for len in (1..=max_check).rev() {
            let slice: String = chars[i..i + len].iter().collect();
            if is_word_in_map(&slice, map) {
                best_len = len;
                break;
            }
        }
        if best_len == 0 {
            best_len = 1;
        }

        let word: String = chars[i..i + best_len].iter().collect();
        let py = word_pinyin(&word, map);
        segmented.push(word);
        pinyin_parts.push(py);
        i += best_len;
    }

    // Compact: no space before punctuation, space after/between
    let mut pinyin_compact = String::new();
    for (i, part) in pinyin_parts.iter().enumerate() {
        if i > 0 {
            let curr_is_punct = part.chars().all(|c| !c.is_ascii_alphanumeric());
            if !curr_is_punct {
                pinyin_compact.push(' ');
            }
        }
        pinyin_compact.push_str(part);
    }

    (segmented.join(" "), pinyin_compact)
}

pub(crate) async fn convert_handler(
    State(_): State<WebState>,
    Json(req): Json<ConvertRequest>,
) -> Json<ConvertResponse> {
    match req.mode.as_str() {
        "s2t" => {
            ensure_s2t_loaded();
            let map = get_s2t_map();
            let max_len = get_s2t_max_len();
            Json(ConvertResponse { result: convert_longest_match(&req.text, map, max_len), segmented: None, pinyin: None })
        }
        "t2s" => {
            ensure_s2t_loaded();
            let map = get_t2s_map();
            let max_len = get_t2s_max_len();
            Json(ConvertResponse { result: convert_longest_match(&req.text, map, max_len), segmented: None, pinyin: None })
        }
        _ => {
            let map = get_chinese_word_map();
            if map.is_empty() {
                return Json(ConvertResponse { result: req.text.clone(), segmented: None, pinyin: None });
            }
            let (segmented, pinyin) = segment_and_convert_pinyin(&req.text, map);
            Json(ConvertResponse { result: String::new(), segmented: Some(segmented), pinyin: Some(pinyin) })
        }
    }
}
