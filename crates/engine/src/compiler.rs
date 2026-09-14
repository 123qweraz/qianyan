use crate::file_lock;
use crate::trie::{TRIE_MAGIC, TRIE_VERSION};
use fst::MapBuilder;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use std::time::SystemTime;
use walkdir::WalkDir;

pub fn check_and_compile_all() -> Result<(), Box<dyn std::error::Error>> {
    let root = qianyan_ime_core::utils::find_project_root();
    fs::create_dir_all(root.join("data"))?;
    println!("[Compiler] 正在扫描 dicts 目录...");

    if let Ok(entries) = fs::read_dir(root.join("dicts")) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                let dir_name = entry.file_name().to_string_lossy().to_string();

                // stroke 方案在 dicts 中可能存在也可能不存在（源文件在 chinese/chars），额外处理
                let src_path = root.join("dicts").join(&dir_name);
                let out_dir = root.join("data").join(&dir_name);

                println!("[Compiler] 检查方案: {}", dir_name);
                fs::create_dir_all(&out_dir)?;

                let trie_dat = out_dir.join("trie.data");
                if should_compile(&src_path, &trie_dat) {
                    println!("[Compiler] 方案 [{}] 需要编译，正在执行...", dir_name);
                    let is_english = dir_name.contains("english");
                    let skip_reverse = is_english || dir_name.contains("japanese");
                    let start = std::time::Instant::now();
                    let trie_out = out_dir.join("trie");
                    compile_dict_for_path(&src_path.to_string_lossy(), &trie_out.to_string_lossy(), is_english, skip_reverse, None, None, None)?;
                    println!(
                        "[Compiler] 方案 [{}] 编译完成，耗时 {:?}",
                        dir_name,
                        start.elapsed()
                    );

                } else {
                    println!("[Compiler] 方案 [{}] 已是最新，跳过。", dir_name);
                    let lock_path = trie_dat.with_extension("data.lock");
                    if !lock_path.exists() {
                        File::create(&lock_path)?;
                    }
                }
            }
        }
    }

    // stroke 方案：从 chinese/chars 编译，用 stroke_aux 作为 key（不依赖 dicts/stroke/ 目录）
    {
        let src_path = root.join("dicts/chinese/chars");
        let out_dir = root.join("data/stroke");
        println!("[Compiler] 检查方案: stroke（从 chinese/chars 编译）");
        fs::create_dir_all(&out_dir)?;
        let trie_dat = out_dir.join("trie.data");
        if should_compile(&src_path, &trie_dat) {
            println!("[Compiler] 方案 [stroke] 需要编译，正在执行...");
            let start = std::time::Instant::now();
            compile_dict_for_path(&src_path.to_string_lossy(), &out_dir.join("trie").to_string_lossy(), false, false, Some("stroke_aux"), None, Some("trad"))?;
            println!("[Compiler] 方案 [stroke] 编译完成，耗时 {:?}", start.elapsed());
        } else {
            println!("[Compiler] 方案 [stroke] 已是最新，跳过。");
            let lock_path = trie_dat.with_extension("data.lock");
            if !lock_path.exists() {
                File::create(&lock_path)?;
            }
        }
    }

    // bihua 方案：从 chinese/chars 编译，用完整笔顺 strokes 字段作为 key（逐笔笔画输入）
    {
        let src_path = root.join("dicts/chinese/chars");
        let out_dir = root.join("data/bihua");
        println!("[Compiler] 检查方案: bihua（逐笔笔画，从 chinese/chars 编译）");
        fs::create_dir_all(&out_dir)?;
        let trie_dat = out_dir.join("trie.data");
        if should_compile(&src_path, &trie_dat) {
            println!("[Compiler] 方案 [bihua] 需要编译，正在执行...");
            let start = std::time::Instant::now();
            compile_dict_for_path(&src_path.to_string_lossy(), &out_dir.join("trie").to_string_lossy(), false, false, Some("strokes"), None, Some("trad"))?;
            println!("[Compiler] 方案 [bihua] 编译完成，耗时 {:?}", start.elapsed());
        } else {
            println!("[Compiler] 方案 [bihua] 已是最新，跳过。");
            let lock_path = trie_dat.with_extension("data.lock");
            if !lock_path.exists() {
                File::create(&lock_path)?;
            }
        }
    }

    // pybi 方案：从 chinese/chars 编译，key=拼音，单字的完整笔顺存入 stroke_aux（供拼音+笔画筛选）
    {
        let src_path = root.join("dicts/chinese/chars");
        let out_dir = root.join("data/pybi");
        println!("[Compiler] 检查方案: pybi（拼音+逐笔笔画，从 chinese/chars 编译）");
        fs::create_dir_all(&out_dir)?;
        let trie_dat = out_dir.join("trie.data");
        if should_compile(&src_path, &trie_dat) {
            println!("[Compiler] 方案 [pybi] 需要编译，正在执行...");
            let start = std::time::Instant::now();
            compile_dict_for_path(&src_path.to_string_lossy(), &out_dir.join("trie").to_string_lossy(), false, true, None, Some("strokes"), Some("trad"))?;
            println!("[Compiler] 方案 [pybi] 编译完成，耗时 {:?}", start.elapsed());
        } else {
            println!("[Compiler] 方案 [pybi] 已是最新，跳过。");
            let lock_path = trie_dat.with_extension("data.lock");
            if !lock_path.exists() {
                File::create(&lock_path)?;
            }
        }
    }

    Ok(())
}

fn should_compile(src_dir: &Path, target_file: &Path) -> bool {
    if !target_file.exists() {
        return true;
    }

    // 检查版本头，如果版本不对也需要重新编译
    if let Ok(mut file) = File::open(target_file) {
        use std::io::Read;
        let mut magic = [0u8; 4];
        let mut version_bytes = [0u8; 4];
        if file.read_exact(&mut magic).is_ok() && &magic == TRIE_MAGIC {
            if file.read_exact(&mut version_bytes).is_ok() {
                let version = u32::from_le_bytes(version_bytes);
                if version != TRIE_VERSION {
                    return true;
                }
            } else {
                return true;
            }
        } else {
            // 旧版本没有头，或者魔法数字不对
            return true;
        }
    }

    let target_mtime = target_file
        .metadata()
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);

    // 检查目录本身的修改时间 (只有当目录比目标文件新时才考虑进一步检查)
    if let Ok(m) = src_dir.metadata().and_then(|m| m.modified()) {
        if m > target_mtime {
            // 目录变了不一定代表内容变了，继续深挖
        } else {
            // 目录都没变，肯定没加减文件
            return false;
        }
    }

    let mut max_src_mtime = SystemTime::UNIX_EPOCH;
    for entry in WalkDir::new(src_dir).into_iter().filter_map(|e| e.ok()) {
        if entry.path().is_file() {
            let ext = entry
                .path()
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            if ext == "json" || ext == "yaml" {
                if let Ok(mtime) = entry.path().metadata().and_then(|m| m.modified()) {
                    if mtime > max_src_mtime {
                        max_src_mtime = mtime;
                    }
                }
            }
        }
    }

    // 只有源文件明确比编译产物新时（允许 1 秒以内的误差，解决某些打包工具的时间戳舍入问题）
    if let Ok(duration) = max_src_mtime.duration_since(target_mtime) {
        duration.as_secs() >= 1
    } else {
        false // 源文件比产物旧或时间一致
    }
}

fn compile_dict_for_path(
    src_dir: &str,
    out_stem: &str,
    is_english: bool,
    skip_reverse: bool,
    key_field: Option<&str>,
    strokes_as_aux: Option<&str>,
    skip_category: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut entries: BTreeMap<String, Vec<DictEntry>> = BTreeMap::new();
    let simp_trad = collect_simp_to_trad_map(src_dir);
    for entry in WalkDir::new(src_dir).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            if path.file_name().and_then(|n| n.to_str()) == Some("punctuation.json") {
                continue;
            }
            process_json_file(path, &mut entries, is_english, key_field, strokes_as_aux, skip_category, &simp_trad)?;
        } else if path.extension().is_some_and(|ext| ext == "yaml") {
            process_yaml_file(path, &mut entries)?;
        }
    }
    // 构建倒排 FST: word → pinyin (用于 has_word_in_dict / lookup_pinyin)
    // English/Japanese 不需要（键即是词本身，或无需反向查找）
    if !skip_reverse {
        let word_map_path = format!("{}.word_map", out_stem);
        let pinyin_data_path = format!("{}.pinyin_data", out_stem);
        let mut word_pinyin_map: BTreeMap<&str, &str> = BTreeMap::new();
        for (pinyin, pairs) in &entries {
            for entry in pairs {
                word_pinyin_map.entry(&entry.word).or_insert(pinyin);
            }
        }
        let mut pinyin_writer = BufWriter::new(File::create(&pinyin_data_path)?);
        let mut word_map_builder = MapBuilder::new(File::create(&word_map_path)?)?;
        let mut offset = 0u64;
        for (word, pinyin) in &word_pinyin_map {
            let pinyin_bytes = pinyin.as_bytes();
            pinyin_writer.write_all(pinyin_bytes)?;
            let value = (offset << 32) | (pinyin_bytes.len() as u64);
            word_map_builder.insert(word, value)?;
            offset += pinyin_bytes.len() as u64;
        }
        word_map_builder.finish()?;
    }

    write_binary_dict(
        &format!("{}.index", out_stem),
        &format!("{}.data", out_stem),
        entries,
    )
}

struct DictEntry {
    word: String,
    trad: String,
    tone: String,
    en: String,
    stroke_aux: String,
    weight: u32,
    flags: u8, // 单字按 category 置位；多字词/未分类词条 = 0（恒显示）
}

/// 生僻位：level-4 | level-5
pub(crate) const RARE_BITS: u8 = 0x18;

/// 字符 category -> flags 位。仅单字分类；词条（词/用户词等）不赋值，保持 0。
pub(crate) fn category_flag(category: &str) -> u8 {
    match category {
        "level-1" => 0x01,
        "level-2" => 0x02,
        "level-3" => 0x04,
        "level-4" => 0x08,
        "level-5" => 0x10,
        "trad" => 0x20,
        _ => 0,
    }
}

/// 收集 简体→繁体 单字映射：单字条目的 trad 字段即其繁体（无区别时等于自身跳过）。
/// 词条（多字词）删掉冗余 trad 字段后，编译时按字查此表补出繁体，保证繁体输出不回归。
fn collect_simp_to_trad_map(src_dir: &str) -> std::collections::HashMap<String, String> {
    let mut m = std::collections::HashMap::new();
    for entry in WalkDir::new(src_dir).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            let Ok(json) = std::fs::read_to_string(path).map(|s| serde_json::from_str::<Value>(&s)).map(|r| r.ok()) else {
                continue;
            };
            let Some(json) = json else { continue; };
            let Some(obj) = json.as_object() else { continue; };
            for val in obj.values() {
                let Some(arr) = val.as_array() else { continue; };
                for v in arr {
                    let Some(o) = v.as_object() else { continue; };
                    let Some(c) = o.get("char").and_then(|c| c.as_str()) else { continue; };
                    if c.chars().count() != 1 {
                        continue;
                    }
                    let Some(t) = o.get("trad").and_then(|t| t.as_str()) else { continue; };
                    if t != c {
                        m.insert(c.to_string(), t.to_string());
                    }
                }
            }
        }
    }
    m
}

fn process_json_file(
    path: &Path,
    entries: &mut BTreeMap<String, Vec<DictEntry>>,
    is_english: bool,
    key_field: Option<&str>,
    strokes_as_aux: Option<&str>,
    skip_category: Option<&str>,
    simp_trad: &std::collections::HashMap<String, String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = File::open(path)?;
    let json: Value = serde_json::from_reader(file)?;
    if let Some(obj) = json.as_object() {
        for (key, val) in obj {
            // 强制转为小写，确保搜索一致性
            let normalized_key = key.to_lowercase();
            if let Some(arr) = val.as_array() {
                if is_english {
                    let en_hint = arr
                        .iter()
                        .filter_map(|v| v.as_str())
                        .next()
                        .unwrap_or("")
                        .to_string();
                    entries
                        .entry(normalized_key.clone())
                        .or_default()
                        .push(DictEntry {
                            word: key.clone(),
                            trad: key.clone(),
                            tone: String::new(),
                            en: en_hint,
                            stroke_aux: String::new(),
                            weight: 0,
                            flags: 0,
                        });
                } else {
                    for v in arr {
                        if let Some(s) = v.as_str() {
                            entries
                                .entry(normalized_key.clone())
                                .or_default()
                                .push(DictEntry {
                                    word: s.to_string(),
                                    trad: s.to_string(),
                                    tone: String::new(),
                                    en: String::new(),
                                    stroke_aux: String::new(),
                                    weight: 0,
                                    flags: 0,
                                });
                        } else if let Some(o) = v.as_object() {
                            if let Some(c) = o.get("char").and_then(|c| c.as_str()) {
                                // 繁体：优先取条目自带 trad 字段；多字词缺失时按字查 简→繁 表
                                let raw_trad = o.get("trad").and_then(|t| t.as_str());
                                let trad = match raw_trad {
                                    Some(t) => t.to_string(),
                                    None if c.chars().count() > 1 => c.chars()
                                        .map(|ch| simp_trad.get(&ch.to_string()).cloned().unwrap_or_else(|| ch.to_string()))
                                        .collect::<String>(),
                                    None => c
                                        .chars()
                                        .map(|ch| simp_trad.get(&ch.to_string()).cloned().unwrap_or_else(|| ch.to_string()))
                                        .collect::<String>(),
                                };
                                let en_hint = o.get("en").and_then(|e| e.as_str()).unwrap_or("");
                                let tone_hint =
                                    o.get("tone").and_then(|t| t.as_str()).unwrap_or("");
                                // 当需以完整笔顺作为 aux 时（pybi），优先取 strokes 字段
                                let stroke_aux = if strokes_as_aux.is_some() {
                                    let s = o.get("strokes")
                                        .and_then(|s| s.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    // pybi 需要完整笔顺做筛选，无笔顺的僻字直接跳过
                                    if s.is_empty() {
                                        continue;
                                    }
                                    s
                                } else {
                                    o.get("stroke_aux")
                                        .and_then(|s| s.as_str())
                                        .unwrap_or("")
                                        .to_string()
                                };
                                let weight =
                                    o.get("weight").and_then(|w| w.as_u64()).unwrap_or(0) as u32;
                                let category = o
                                    .get("category")
                                    .and_then(|s| s.as_str())
                                    .unwrap_or("");
                                // 简体主词典以外的方案（stroke/bihua/pybi）跳过繁体扩展候选
                                if skip_category == Some(category) {
                                    continue;
                                }
                                // 仅单字按 category 置位；多字词始终 flags=0（恒显示）
                                let flags = if c.chars().count() == 1 {
                                    category_flag(category)
                                } else {
                                    0
                                };

                                // 当指定 key_field 时，用该字段的值作为搜索 key
                                let trie_key = if let Some(field) = key_field {
                                    let k = o.get(field)
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_lowercase();
                                    if k.is_empty() {
                                        continue; // 跳过没有该字段的条目
                                    }
                                    k
                                } else {
                                    normalized_key.clone()
                                };

                                entries.entry(trie_key).or_default().push(
                                    DictEntry {
                                        word: c.to_string(),
                                        trad: trad.to_string(),
                                        tone: tone_hint.to_string(),
                                        en: en_hint.to_string(),
                                        stroke_aux: stroke_aux.to_string(),
                                        weight,
                                        flags,
                                    },
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn process_yaml_file(
    path: &Path,
    entries: &mut BTreeMap<String, Vec<DictEntry>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut in_data = false;
    for line in reader.lines().map_while(Result::ok) {
        if !in_data {
            if line.starts_with("...") {
                in_data = true;
            }
            continue;
        }
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() >= 2 {
            let word = parts[0].to_string();
            // 强制转为小写并移除空格
            let pinyin = parts[1].replace(' ', "").to_lowercase();
            let weight = if parts.len() >= 3 {
                parts[2].parse::<u32>().unwrap_or(0)
            } else {
                0
            };
            entries.entry(pinyin).or_default().push(DictEntry {
                word: word.clone(),
                trad: word,
                tone: String::new(),
                en: String::new(),
                stroke_aux: String::new(),
                weight,
                flags: 0,
            });
        }
    }
    Ok(())
}

fn write_binary_dict(
    idx_path: &str,
    dat_path: &str,
    entries: BTreeMap<String, Vec<DictEntry>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let tmp_idx = format!("{}.tmp", idx_path);
    let tmp_dat = format!("{}.tmp", dat_path);

    // Acquire exclusive lock to prevent readers from mmaping during replacement.
    let lock_path = std::path::Path::new(dat_path).with_extension("data.lock");
    let _write_lock = file_lock::lock_exclusive(&lock_path)
        .map_err(|e| format!("[Compiler] 无法获取排他文件锁 {}: {}", lock_path.display(), e))?;

    {
        let mut data_writer = BufWriter::new(File::create(&tmp_dat)?);
        let mut index_builder = MapBuilder::new(File::create(&tmp_idx)?)?;

        // 写入头部
        data_writer.write_all(TRIE_MAGIC)?;
        data_writer.write_all(&TRIE_VERSION.to_le_bytes())?;

        let mut current_offset = 8u64;
        for (pinyin, mut pairs) in entries {
            // 同词去重：首次出现保留；若同一词同时存在生僻(level-4/5)与普通版本，
            // 普通版本覆盖生僻版本（如 chars/trads.json 以 category=trad
            // 非生僻候选注入，覆盖其在 level-4/5 中的生僻形式）
            let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
            let mut deduped: Vec<DictEntry> = Vec::with_capacity(pairs.len());
            for e in pairs {
                match seen.get(&e.word) {
                    None => {
                        seen.insert(e.word.clone(), deduped.len());
                        deduped.push(e);
                    }
                    Some(&i) => {
                        if deduped[i].flags & RARE_BITS != 0 && e.flags & RARE_BITS == 0 {
                            deduped[i] = e;
                        }
                    }
                }
            }
            pairs = deduped;
            // v3: 块内按权重降序，供搜索端对低权重尾部提前终止
            pairs.sort_unstable_by_key(|e| std::cmp::Reverse(e.weight));

            index_builder.insert(&pinyin, current_offset)?;
            let mut block = Vec::new();
            block.extend_from_slice(&(pairs.len() as u32).to_le_bytes());
            for entry in pairs {
                let w_bytes = entry.word.as_bytes();
                let tr_bytes = entry.trad.as_bytes();
                let t_bytes = entry.tone.as_bytes();
                let e_bytes = entry.en.as_bytes();
                let s_bytes = entry.stroke_aux.as_bytes();

                block.extend_from_slice(&(w_bytes.len() as u16).to_le_bytes());
                block.extend_from_slice(w_bytes);
                block.extend_from_slice(&(tr_bytes.len() as u16).to_le_bytes());
                block.extend_from_slice(tr_bytes);
                block.extend_from_slice(&(t_bytes.len() as u16).to_le_bytes());
                block.extend_from_slice(t_bytes);
                block.extend_from_slice(&(e_bytes.len() as u16).to_le_bytes());
                block.extend_from_slice(e_bytes);
                block.extend_from_slice(&(s_bytes.len() as u16).to_le_bytes());
                block.extend_from_slice(s_bytes);
                block.extend_from_slice(&entry.weight.to_le_bytes());
                block.push(entry.flags);
            }
            data_writer.write_all(&block)?;
            current_offset += block.len() as u64;
        }
        index_builder.finish()?;
    }

    // Atomically replace files. On Linux, rename is safe even if old file is mmap'd
    // (the old inode remains accessible to existing mmaps). On Windows, the exclusive
    // lock above prevents readers from mmaping during replacement.
    // We never use fs::copy as fallback — that would overwrite the same inode and
    // corrupt existing mmap'd data.
    if let Err(e) = fs::rename(&tmp_idx, idx_path) {
        log::error!("[Compiler] 重命名索引文件失败: {} — 词库文件可能受损", e);
        // Try remove + rename as fallback on platforms where rename-to-existing fails
        let _ = fs::remove_file(idx_path);
        if let Err(e2) = fs::rename(&tmp_idx, idx_path) {
            log::error!("[Compiler] 重命名索引文件失败(重试后): {}", e2);
            return Err(e2.into());
        }
    }
    if let Err(e) = fs::rename(&tmp_dat, dat_path) {
        log::error!("[Compiler] 重命名数据文件失败: {} — 词库文件可能受损", e);
        let _ = fs::remove_file(dat_path);
        if let Err(e2) = fs::rename(&tmp_dat, dat_path) {
            log::error!("[Compiler] 重命名数据文件失败(重试后): {}", e2);
            return Err(e2.into());
        }
    }

    let _ = fs::remove_file(tmp_idx);
    let _ = fs::remove_file(tmp_dat);
    Ok(())
}
