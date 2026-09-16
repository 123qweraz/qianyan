use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// 内置解析引擎 id（编译进程序的“解析引擎”）。
/// 方案文件通过 engine 选择一款引擎，引擎只负责按类型解析输入串 + 访问码表，
/// 不认识任何具体的词库名。
pub const KNOWN_ENGINES: &[&str] = &["pinyin", "english", "japanese", "stroke", "bihua", "pybi"];

/// 注释文档配置（附属于某个方案）：
/// 方案选择的“注释文档”可以是一份英文/日文/任何自定义码表释义，文件放哪都行。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SchemeAnnotation {
    /// 注释源文件路径（TSV/JSON，绝对路径或相对项目根）。空 = 用全局默认规则。
    pub file: String,
    /// 显式命名这张注释表（可空）：优先级最高，直接当作表名使用。
    pub table_name: String,
    /// 仅供界面标注：如 english / japanese / 自定义码表…
    pub doc_type: String,
    /// 该方案彻底关闭注释显示
    pub disabled: bool,
}

/// 一个输入方案 = 一个 JSON 文件（configs/schemes/<id>.json）。
/// 程序只提供解析引擎（engine）；码表（dict）与注释文档（annotation）完全由用户配置。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SchemeDefinition {
    /// 全局唯一 id（= 文件名；托盘名 / default_profile / profile_keys 都引用它）
    pub id: String,
    /// 显示名
    pub label: String,
    /// 解析引擎 id，见 KNOWN_ENGINES
    pub engine: String,
    /// 码表：dicts/<dict> 目录（编译到 data/<dict>/trie）。
    /// 运行期该字典以方案 id 挂载（次要引擎如需共享另一份词库，用 aux_dicts）。
    pub dict: String,
    /// 候选显示模板字段；None = 用全局默认模板
    pub display: Option<Vec<String>>,
    /// 选词辅助码来源；None = 用全局默认
    pub aux_filter: Option<crate::config::AuxFilterMode>,
    /// 注释文档
    pub annotation: SchemeAnnotation,
    /// 附加词库（按 id 挂载，如 pybi → ["chinese"]）
    pub aux_dicts: Vec<String>,
}

impl Default for SchemeDefinition {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            engine: "pinyin".to_string(),
            dict: "chinese".to_string(),
            display: None,
            aux_filter: None,
            annotation: SchemeAnnotation::default(),
            aux_dicts: Vec::new(),
        }
    }
}

/// 扫描 configs/schemes/*.json 得到的方案注册表。
/// 解析/校验失败的条目进入 errors（启动不中断，设置页标红提示）。
#[derive(Debug, Default, Clone)]
pub struct SchemeRegistry {
    pub schemes: Vec<SchemeDefinition>,
    pub errors: Vec<String>,
}

fn lang_of_table(table: &str) -> String {
    table
        .rsplit_once('-')
        .map(|(_, l)| l.to_string())
        .unwrap_or_default()
}

impl SchemeDefinition {
    /// 注释表名 + 目标语言字段键（供候选模板字段使用）。
    /// 显式 table_name > 文件路径自动推导 > 空。
    pub fn annotation_table(&self, annotation_sources: &HashMap<String, String>) -> (String, String) {
        if self.annotation.disabled {
            return (String::new(), String::new());
        }
        if !self.annotation.table_name.is_empty() {
            let t = self.annotation.table_name.clone();
            let lang = lang_of_table(&t);
            return (t, lang);
        }
        if !self.annotation.file.is_empty() {
            let t =
                crate::utils::annotation_table_name_for_path(&self.annotation.file, annotation_sources);
            let lang = lang_of_table(&t);
            return (t, lang);
        }
        (String::new(), String::new())
    }
}

impl SchemeRegistry {
    pub fn load(dir: &Path) -> Self {
        let mut schemes = Vec::new();
        let mut errors = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                match std::fs::read_to_string(&path) {
                    Ok(text) => match serde_json::from_str::<SchemeDefinition>(&text) {
                        Ok(mut def) => {
                            if def.id.is_empty() {
                                def.id = path
                                    .file_stem()
                                    .and_then(|s| s.to_str())
                                    .unwrap_or("")
                                    .to_string();
                            }
                            if def.id.is_empty() {
                                errors.push(format!("{}: id 为空", path.display()));
                                continue;
                            }
                            if def.dict.is_empty() {
                                errors.push(format!("{}: dict 为空", path.display()));
                            }
                            if !KNOWN_ENGINES.contains(&def.engine.as_str()) {
                                errors.push(format!(
                                    "{}: 未知引擎 {:?}（可用：{}）",
                                    path.display(),
                                    def.engine,
                                    KNOWN_ENGINES.join(", ")
                                ));
                            }
                            schemes.push(def);
                        }
                        Err(e) => errors.push(format!("{}: {}", path.display(), e)),
                    },
                    Err(e) => errors.push(format!("{}: {}", path.display(), e)),
                }
            }
        }
        schemes.sort_by(|a, b| a.id.cmp(&b.id));
        Self { schemes, errors }
    }

    pub fn get(&self, id: &str) -> Option<&SchemeDefinition> {
        self.schemes.iter().find(|s| s.id == id)
    }

    pub fn into_map(self) -> HashMap<String, SchemeDefinition> {
        self.schemes.into_iter().map(|s| (s.id.clone(), s)).collect()
    }
}

/// 由方案清单生成 files.profiles（托盘/设置页的启用列表数据源）。
pub fn profiles_from_schemes(defs: &[SchemeDefinition]) -> Vec<crate::config::Profile> {
    defs.iter()
        .map(|s| crate::config::Profile {
            name: s.id.clone(),
            path: format!("data/{}/trie", s.dict),
        })
        .collect()
}

/// 内置方案默认表：(id, label, engine, dict, aux_dicts)
pub fn builtin_scheme_specs() -> Vec<(&'static str, &'static str, &'static str, &'static str, &'static [&'static str])> {
    vec![
        ("chinese", "拼音", "pinyin", "chinese", &[]),
        ("english", "英文", "english", "english", &[]),
        ("japanese", "日语", "japanese", "japanese", &[]),
        ("stroke", "笔顺码", "stroke", "stroke", &[]),
        ("bihua", "逐笔笔画", "bihua", "bihua", &[]),
        ("pybi", "拼音+笔画", "pybi", "pybi", &["chinese"]),
    ]
}

/// 写方案文件到 configs/schemes/<id>.json
pub fn save_scheme(def: &SchemeDefinition, dir: &std::path::Path) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(dir)?;
    let text = serde_json::to_string_pretty(def)?;
    std::fs::write(dir.join(format!("{}.json", def.id)), text)
}

pub fn delete_scheme(id: &str, dir: &std::path::Path) -> bool {
    std::fs::remove_file(dir.join(format!("{}.json", id))).is_ok()
}

/// 首启迁移：目录中没有任何方案文件时，从内置 6 方案生成默认方案文件，
/// 并把历史上散落在全局 pinyin.json 的按方案覆盖（候选模板/辅助码/注释文件）迁入对应方案。
/// 返回 true = 本次执行了迁移（caller 应重新 load）。
pub fn ensure_default_schemes(
    dir: &std::path::Path,
    input: &crate::config::Input,
) -> Result<bool, std::io::Error> {
    if let Ok(entries) = std::fs::read_dir(dir) {
        let has_json = entries
            .flatten()
            .any(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"));
        if has_json {
            return Ok(false);
        }
    }
    for (id, label, engine, dict, aux) in builtin_scheme_specs() {
        let mut def = SchemeDefinition {
            id: id.to_string(),
            label: label.to_string(),
            engine: engine.to_string(),
            dict: dict.to_string(),
            aux_dicts: aux.iter().map(|s| s.to_string()).collect(),
            ..SchemeDefinition::default()
        };
        if let Some(fields) = input.candidate_display.per_profile.get(id) {
            if !fields.is_empty() {
                def.display = Some(fields.clone());
            }
        }
        def.aux_filter = input.filter_aux_per_profile.get(id).copied();
        if let Some(path) = input.annotation_files.get(id) {
            if path.is_empty() {
                def.annotation.disabled = true;
            } else {
                def.annotation.file = path.clone();
            }
        }
        save_scheme(&def, dir)?;
    }
    Ok(true)
}

#[cfg(test)]
mod scheme_def_tests {
    use super::*;

    fn fn_offset(v: Option<&[String]>) -> Vec<&str> {
        v.map(|x| x.iter().map(|s| s.as_str()).collect()).unwrap_or_default()
    }

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "qianyan_scheme_test_{}_{}",
            std::process::id(),
            label
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn save_load_roundtrip_and_id_fallback() {
        let dir = temp_dir("roundtrip");
        let mut def = SchemeDefinition::default();
        def.id = "mypy".into();
        def.label = "我的拼音".into();
        def.engine = "pinyin".into();
        def.dict = "chinese".into();
        def.display = Some(vec!["char".into(), "en".into()]);
        def.annotation.file = "/abs/x.tsv".into();
        def.annotation.disabled = false;
        save_scheme(&def, &dir).unwrap();

        let reg = SchemeRegistry::load(&dir);
        assert_eq!(reg.errors.len(), 0);
        let loaded = reg.get("mypy").unwrap();
        assert_eq!(loaded.label, "我的拼音");
        assert_eq!(loaded.engine, "pinyin");
        assert_eq!(fn_offset(loaded.display.as_deref()), vec!["char", "en"]);

        // JSON 缺 id → 回退到文件名
        std::fs::write(
            dir.join("no-id.json"),
            r#"{"id":"","label":"无名","engine":"bihua","dict":"bihua"}"#,
        )
        .unwrap();
        let reg2 = SchemeRegistry::load(&dir);
        assert_eq!(reg2.get("no-id").unwrap().label, "无名");
        assert_eq!(reg2.get("no-id").unwrap().id, "no-id");

        delete_scheme("mypy", &dir);
        assert!(!dir.join("mypy.json").exists());
        delete_scheme("no-id", &dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn registry_reports_bad_engine_and_bad_json() {
        let dir = temp_dir("errors");
        std::fs::write(dir.join("a.json"), r#"{"id":"a","label":"A","engine":"yoyo","dict":"x"}"#).unwrap();
        std::fs::write(dir.join("b.json"), "not-json").unwrap();
        std::fs::write(dir.join("c.json"), r#"{"id":"c","label":"C","engine":"pinyin","dict":"chinese"}"#).unwrap();
        let reg = SchemeRegistry::load(&dir);
        assert_eq!(reg.schemes.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["a", "c"]);
        assert_eq!(reg.errors.len(), 2, "unknown engine + bad json: {:#?}", reg.errors);
        assert!(reg.errors.iter().any(|e| e.contains("yoyo")));
        assert_eq!(reg.get("c").unwrap().dict, "chinese");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn annotation_table_precedence() {
        let sources: HashMap<String, String> =
            HashMap::from([("zh-fr".to_string(), "/named/mydict.tsv".to_string())]);
        let mut def = SchemeDefinition::default();

        // disabled 最高：即使有文件/表名也返回空
        def.annotation.disabled = true;
        def.annotation.table_name = "zh-de".into();
        def.annotation.file = "/x/zh-jp.tsv".into();
        assert_eq!(def.annotation_table(&sources), (String::new(), String::new()));

        // 显式表名：优先于文件
        def.annotation.disabled = false;
        assert_eq!(
            def.annotation_table(&sources),
            ("zh-de".to_string(), "de".to_string())
        );

        // 文件路径推导（ASCII 主干 → 表名 + 语言键）
        def.annotation.table_name.clear();
        assert_eq!(
            def.annotation_table(&sources),
            ("zh-jp".to_string(), "jp".to_string())
        );

        // 文件命中 annotation_sources 值 → 用其键名
        def.annotation.file = "/named/mydict.tsv".into();
        assert_eq!(
            def.annotation_table(&sources),
            ("zh-fr".to_string(), "fr".to_string())
        );

        // 都没填 → 空（回退全局规则）
        def.annotation.file.clear();
        assert_eq!(def.annotation_table(&sources), (String::new(), String::new()));
    }

    #[test]
    fn ensure_default_schemes_writes_builtins_and_migrates() {
        use crate::config::AuxFilterMode;
        let dir = temp_dir("migrate");
        let mut input = crate::config::Config::default_config().input;
        input
            .candidate_display
            .per_profile
            .insert("chinese".into(), vec!["char".into(), "jp".into()]);
        input.filter_aux_per_profile.insert("english".into(), AuxFilterMode::English);
        input
            .annotation_files
            .insert("japanese".into(), "/custom/jp-zh.tsv".into());

        assert!(ensure_default_schemes(&dir, &input).unwrap());
        assert!(!ensure_default_schemes(&dir, &input).unwrap(), "second run = no-op");

        let reg = SchemeRegistry::load(&dir);
        assert_eq!(reg.errors.len(), 0);
        let ids: Vec<_> = reg.schemes.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["bihua", "chinese", "english", "japanese", "pybi", "stroke"]);

        let chinese = reg.get("chinese").unwrap();
        assert_eq!(fn_offset(chinese.display.as_deref()), vec!["char", "jp"]);
        assert_eq!(chinese.dict, "chinese");
        assert_eq!(reg.get("english").unwrap().aux_filter, Some(AuxFilterMode::English));
        assert_eq!(reg.get("stroke").unwrap().aux_filter, None);
        let jp = reg.get("japanese").unwrap();
        assert_eq!(jp.annotation.file, "/custom/jp-zh.tsv");
        assert!(!jp.annotation.disabled);
        assert_eq!(reg.get("pybi").unwrap().aux_dicts, vec!["chinese"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn builtin_specs_produce_profiles_and_known_engines() {
        let specs = builtin_scheme_specs();
        assert_eq!(specs.len(), 6);
        for (id, _, engine, dict, aux) in &specs {
            assert!(KNOWN_ENGINES.contains(engine), "{}", engine);
            let def = SchemeDefinition {
                id: id.to_string(),
                engine: engine.to_string(),
                dict: dict.to_string(),
                aux_dicts: aux.iter().map(|s| s.to_string()).collect(),
                ..SchemeDefinition::default()
            };
            let profiles = profiles_from_schemes(std::slice::from_ref(&def));
            assert_eq!(profiles.len(), 1);
            assert_eq!(profiles[0].name, *id);
            assert_eq!(profiles[0].path, format!("data/{}/trie", dict));
        }
    }
}