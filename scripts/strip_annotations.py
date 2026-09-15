#!/usr/bin/env python3
"""将"词条内嵌注释"拆分为独立注释表（青简式）。

- dicts/chinese/**、dicts/chinese_extras/**、dicts/japanese/** 中的 "en" 字段
  剥离后汇入 dicts/annotations/en.json（key=词/汉字，value=英文释义）。
- dicts/english/** 的词条译文清空，译文汇入 dicts/annotations/zh.json
  （key=英文词，value=首个中文释义）。基础词典只保留词表。
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


def write_json(path, data):
    tmp = path.with_suffix(".tmp")
    tmp.write_text(
        json.dumps(data, ensure_ascii=False, indent=None, separators=(",", ":")),
        encoding="utf-8",
    )
    tmp.replace(path)


def pick_gloss(glosses):
    """glosses: [(gloss, weight), ...] → 权重最高者；同权重取最长释义。"""
    best = max(glosses, key=lambda g: (g[1], len(g[0])))
    return best[0]


def main():
    ANN_DIR.mkdir(parents=True, exist_ok=True)
    en = {}       # 中文词/日文词 -> 英文释义
    zh = {}       # 英文词 -> 中文释义

    # ── 中文词库：剥离 en -> en.json ──
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
                        glosses = en.setdefault(key, [])
                        glosses.append((gloss, 0))
                    del item["en"]
                    changed = True
            if changed:
                write_json(path, data)
                print(f"[strip] {path.relative_to(ROOT)}")

    # ── 日文词库：剥离 en -> en.json（同 char 多读音取权重最高释义）──
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
                    glosses = en.setdefault(str(char), [])
                    glosses.append((item["en"], weight))
                del item["en"]
                changed = True
        if changed:
            write_json(path, data)
            print(f"[strip] {rel}")

    # ── 英文词库：译文清空 -> zh.json ──
    for rel in ENGLISH_FILES:
        path = ROOT / rel
        if not path.exists():
            continue
        data = json.loads(path.read_text(encoding="utf-8"))
        changed = False
        for word, values in data.items():
            if isinstance(values, list) and values:
                first = next((v for v in values if isinstance(v, str) and v), None)
                if first and word not in zh:
                    zh[word] = first
                data[word] = []
                changed = True
        if changed:
            write_json(path, data)
            print(f"[strip] {rel}")

    # ── 汇总写出（确定性排序，缩进便于审查版本差异）──
    en_out = {k: pick_gloss(v) for k, v in sorted(en.items())}
    zh_out = {k: zh[k] for k in sorted(zh)}
    write_json(ANN_DIR / "en.json", en_out)
    write_json(ANN_DIR / "zh.json", zh_out)
    print(f"[done] en.json: {len(en_out)} 条, zh.json: {len(zh_out)} 条")


if __name__ == "__main__":
    sys.exit(main())