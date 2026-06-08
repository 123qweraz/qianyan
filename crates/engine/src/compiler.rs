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
    fs::create_dir_all("data")?;
    println!("[Compiler] 正在扫描 dicts 目录...");

    if let Ok(entries) = fs::read_dir("dicts") {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                let dir_name = entry.file_name().to_string_lossy().to_string();

                // stroke 方案特殊处理：从 chinese/chars 编译，用 stroke_aux 作为 key
                if dir_name == "stroke" {
                    println!("[Compiler] 检查方案: stroke（从 chinese/chars 编译）");
                    let src_path = "dicts/chinese/chars";
                    let out_dir = "data/stroke";
                    fs::create_dir_all(out_dir)?;
                    let trie_dat = format!("{}/trie.data", out_dir);
                    if should_compile(Path::new(src_path), Path::new(&trie_dat)) {
                        println!("[Compiler] 方案 [stroke] 需要编译，正在执行...");
                        let start = std::time::Instant::now();
                        compile_dict_for_path(src_path, &format!("{}/trie", out_dir), false, Some("stroke_aux"))?;
                        println!(
                            "[Compiler] 方案 [stroke] 编译完成，耗时 {:?}",
                            start.elapsed()
                        );
                    } else {
                        println!("[Compiler] 方案 [stroke] 已是最新，跳过。");
                    }
                    continue;
                }

                let src_path = format!("dicts/{}", dir_name);
                let out_dir = format!("data/{}", dir_name);

                println!("[Compiler] 检查方案: {}", dir_name);
                fs::create_dir_all(&out_dir)?;

                let trie_dat = format!("{}/trie.data", out_dir);
                if should_compile(Path::new(&src_path), Path::new(&trie_dat)) {
                    println!("[Compiler] 方案 [{}] 需要编译，正在执行...", dir_name);
                    let is_english = dir_name.contains("english");
                    let start = std::time::Instant::now();
                    compile_dict_for_path(&src_path, &format!("{}/trie", out_dir), is_english, None)?;
                    println!(
                        "[Compiler] 方案 [{}] 编译完成，耗时 {:?}",
                        dir_name,
                        start.elapsed()
                    );

                } else {
                    println!("[Compiler] 方案 [{}] 已是最新，跳过。", dir_name);
                }
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
    key_field: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut entries: BTreeMap<String, Vec<DictEntry>> = BTreeMap::new();
    for entry in WalkDir::new(src_dir).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            if path.file_name().and_then(|n| n.to_str()) == Some("punctuation.json") {
                continue;
            }
            process_json_file(path, &mut entries, is_english, key_field)?;
        } else if path.extension().is_some_and(|ext| ext == "yaml") {
            process_yaml_file(path, &mut entries)?;
        }
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
    flags: u8, // bit0: 1 = 生僻字 (level-4, level-5)
}

/// 0x01 = 生僻字 (level-4, level-5)
const FLAG_RARE: u8 = 0x01;

fn process_json_file(
    path: &Path,
    entries: &mut BTreeMap<String, Vec<DictEntry>>,
    is_english: bool,
    key_field: Option<&str>,
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
                                let trad = o.get("trad").and_then(|t| t.as_str()).unwrap_or(c);
                                let en_hint = o.get("en").and_then(|e| e.as_str()).unwrap_or("");
                                let tone_hint =
                                    o.get("tone").and_then(|t| t.as_str()).unwrap_or("");
                                let stroke_aux = o
                                    .get("stroke_aux")
                                    .and_then(|s| s.as_str())
                                    .unwrap_or("");
                                let weight =
                                    o.get("weight").and_then(|w| w.as_u64()).unwrap_or(0) as u32;
                                let category = o
                                    .get("category")
                                    .and_then(|s| s.as_str())
                                    .unwrap_or("");
                                let flags = if category == "level-4" || category == "level-5" { FLAG_RARE } else { 0 };

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

    {
        let mut data_writer = BufWriter::new(File::create(&tmp_dat)?);
        let mut index_builder = MapBuilder::new(File::create(&tmp_idx)?)?;

        // 写入头部
        data_writer.write_all(TRIE_MAGIC)?;
        data_writer.write_all(&TRIE_VERSION.to_le_bytes())?;

        let mut current_offset = 8u64;
        for (pinyin, mut pairs) in entries {
            let mut seen = std::collections::HashSet::new();
            pairs.retain(|e| seen.insert(e.word.clone()));

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

    // Windows 兼容处理：如果文件正在被 Mmap 映射，rename 会失败
    #[cfg(target_os = "windows")]
    {
        // 尝试先删除旧文件（通常也会失败，但能触发明确的错误）
        let _ = fs::remove_file(idx_path);
        let _ = fs::remove_file(dat_path);
    }

    if let Err(e) = fs::rename(&tmp_idx, idx_path) {
        log::warn!("[Compiler] 无法重命名索引文件 (可能正在被使用): {}", e);
        // 如果 rename 失败，尝试直接拷贝（虽然通常也会失败，但作为最后尝试）
        let _ = fs::copy(&tmp_idx, idx_path);
    }
    if let Err(e) = fs::rename(&tmp_dat, dat_path) {
        log::warn!("[Compiler] 无法重命名数据文件 (可能正在被使用): {}", e);
        let _ = fs::copy(&tmp_dat, dat_path);
    }

    let _ = fs::remove_file(tmp_idx);
    let _ = fs::remove_file(tmp_dat);
    Ok(())
}
