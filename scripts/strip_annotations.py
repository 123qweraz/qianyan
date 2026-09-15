#!/usr/bin/env python3
"""将"词条内嵌注释"拆分为独立注释表（青简式，TSV 格式）。

- dicts/chinese/**、dicts/chinese_extras/** 中剥离的 "en" → dicts/annotations/zh-en.tsv
  （中文词 → 英文释义；同键取最长释义）。
- dicts/japanese/** 中剥离的 "en" → dicts/annotations/jp-en.tsv
  （日文词 → 英文释义；同键取权重最高读音的释义）。
- dicts/english/** 的译文 → dicts/annotations/en-zh.tsv（键=英文词，统一小写）。
  译文清空，基础词典只保留词表。

TSV 约定：首行 "# 词\t译词" 注释头；每行 "键\t译词"；"#" 开头为注释；无转义，
生成时保证译词不含 tab/换行。
"""
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ANN_DIR = ROOT / "dicts" / "annotations"

CHINESE_GLOBS = [
    "dicts/chinese/**/*.json",
    "dicts/chinese_extras/**/*.json",
]
JAPANESE_FILES = [
    "dicts/japanese/N1.json",
    "dicts/japanese/N2.json",
    "dicts/japanese/N3.json",
    "dicts/japanese/N4.json",
    "dicts/japanese/N5.json",
    "dicts/japanese/cjdict.json",
]
ENGLISH_FILES = [
    "dicts/english/combined_dict.json",
    "dicts/english/dict_5_10s.json",
    "dicts/english/dict_enlt5s.json",
    "dicts/english/linux_commands.json",
]
OUTPUTS = ["zh-en.tsv", "jp-en.tsv", "en-zh.tsv"]


def write_json(path, data):
    tmp = path.with_suffix(".tmp")
    tmp.write_text(
        json.dumps(data, ensure_ascii=False, indent=None, separators=(",", ":")),
        encoding="utf-8",
    )
    tmp.replace(path)


def sanitize(gloss):
    return gloss.replace("\t", " ").replace("\r", " ").replace("\n", " ").strip()


def write_tsv(path, entries, header="# 词\t译词"):
    lines = [header]
    for key in sorted(entries):
        gloss = sanitize(entries[key])
        if not gloss:
            continue
        lines.append(f"{key}\t{gloss}")
    tmp = path.with_suffix(".tmp")
    tmp.write_text("\n".join(lines) + "\n", encoding="utf-8")
    tmp.replace(path)


def main():
    ANN_DIR.mkdir(parents=True, exist_ok=True)
    zh_en = {}   # 中文词 -> [(英文释义, weight)]
    jp_en = {}   # 日文词 -> [(英文释义, weight)]
    en_zh = {}   # 英文词 -> 中文释义

    # ── 中文词库：剥离 en -> zh-en ──
    for pattern in CHINESE_GLOBS:
        for path in sorted(ROOT.glob(pattern)):
            try:
                data = json.loads(path.read_text(encoding="utf-8"))
            except Exception:
                continue
            if not isinstance(data, dict):
                continue
            changed = False
            for values in data.values():
                if not isinstance(values, list):
                    continue
                for item in values:
                    if not isinstance(item, dict) or "en" not in item:
                        continue
                    gloss = item["en"]
                    char = item.get("char")
                    if gloss and char:
                        key = str(char)
                        zh_en.setdefault(key, []).append((gloss, 0))
                    del item["en"]
                    changed = True
            if changed:
                write_json(path, data)
                print(f"[strip] {path.relative_to(ROOT)}")

    # ── 日文词库：剥离 en -> jp-en（同 char 多读音取权重最高释义）──
    for rel in JAPANESE_FILES:
        path = ROOT / rel
        if not path.exists():
            continue
        data = json.loads(path.read_text(encoding="utf-8"))
        changed = False
        for values in data.values():
            if not isinstance(values, list):
                continue
            for item in values:
                if not isinstance(item, dict) or "en" not in item:
                    continue
                char = item.get("char")
                weight = int(item.get("weight") or 0)
                if item["en"] and char:
                    jp_en.setdefault(str(char), []).append((item["en"], weight))
                del item["en"]
                changed = True
        if changed:
            write_json(path, data)
            print(f"[strip] {rel}")

    # ── 英文词库：译文清空 -> en-zh（键统一小写）──
    for rel in ENGLISH_FILES:
        path = ROOT / rel
        if not path.exists():
            continue
        data = json.loads(path.read_text(encoding="utf-8"))
        changed = False
        for word, values in data.items():
            if isinstance(values, list) and values:
                first = next((v for v in values if isinstance(v, str) and v), None)
                key = word.strip().lower()
                if first and key and key not in en_zh:
                    en_zh[key] = first
                data[word] = []
                changed = True
        if changed:
            write_json(path, data)
            print(f"[strip] {rel}")

    # ── 汇总写出（确定性排序）──
    zh_en_out = {k: max(v, key=lambda g: len(g[0]))[0] for k, v in zh_en.items()}
    jp_en_out = {k: max(v, key=lambda g: (g[1], len(g[0])))[0] for k, v in jp_en.items()}
    write_tsv(ANN_DIR / "zh-en.tsv", zh_en_out)
    write_tsv(ANN_DIR / "jp-en.tsv", jp_en_out)
    write_tsv(ANN_DIR / "en-zh.tsv", en_zh)
    for old in ("en.json", "zh.json"):
        (ANN_DIR / old).unlink(missing_ok=True)
    print(
        f"[done] zh-en.tsv: {len(zh_en_out)} 条, jp-en.tsv: {len(jp_en_out)} 条, "
        f"en-zh.tsv: {len(en_zh)} 条"
    )


if __name__ == "__main__":
    sys.exit(main())