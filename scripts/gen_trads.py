#!/usr/bin/env python3
"""从 dicts/chinese/chars/*.json 生成繁体单字词条 trads.json。

背景：繁体字大多以 level-4/5（生僻）形式存在，默认 RareCharMode::CommonOnly
会在候选中直接过滤生僻字，导致打拼音出不了繁体。本脚本把每个繁体单字以
普通候选（flags=0）注入、词频(weight)=1，使其落在该拼音候选项底部，
输入简体读音即可顺带打出繁体字，无须运行时简繁转换。

规则：
- 逐键收集 char≠trad 的单字繁体形式
- 跳过该键下已是"非生僻"候选项的繁体（避免重复）
- 对仅以生僻形式存在的繁体，拷贝其 tone/stroke_aux 作为提示
- 同一键内去重，输出确定性排序，便于 diff

trads.json 会被词典编译器自动并入 data/chinese/profile/trie 主索引。

用法: 在仓库根目录执行  python3 scripts/gen_trads.py
"""
import glob
import json
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CHARS_DIR = os.path.join(ROOT, "dicts", "chinese", "chars")
OUT = os.path.join(ROOT, "dicts", "chinese", "trads.json")
WEIGHT = 1
RARE_CATEGORIES = {"level-4", "level-5"}


def main() -> int:
    non_rare: set[tuple[str, str]] = set()
    rare_meta: dict[tuple[str, str], dict] = {}
    trads: dict[str, set[str]] = {}

    for f in sorted(glob.glob(os.path.join(CHARS_DIR, "*.json"))):
        with open(f, encoding="utf-8") as fh:
            data = json.load(fh)
        for key, val in data.items():
            if not isinstance(val, list):
                continue
            for entry in val:
                if not isinstance(entry, dict):
                    continue
                char = entry.get("char")
                if not isinstance(char, str) or len(char) != 1:
                    continue
                category = entry.get("category", "")
                if category not in RARE_CATEGORIES:
                    non_rare.add((key, char))
                else:
                    rare_meta.setdefault((key, char), entry)
                trad = entry.get("trad", "")
                if isinstance(trad, str) and len(trad) == 1 and trad != char:
                    trads.setdefault(key, set()).add(trad)

    out: dict[str, list[dict]] = {}
    count = 0
    for key in sorted(trads):
        for trad in sorted(trads[key]):
            if (key, trad) in non_rare:
                continue
            meta = rare_meta.get((key, trad))
            entry: dict = {"char": trad, "trad": trad, "weight": WEIGHT}
            if meta is not None:
                tone = meta.get("tone", "")
                if isinstance(tone, str) and tone:
                    entry["tone"] = tone
                sa = meta.get("stroke_aux", "")
                if isinstance(sa, str) and sa:
                    entry["stroke_aux"] = sa
            out.setdefault(key, []).append(entry)
            count += 1

    with open(OUT, "w", encoding="utf-8") as fh:
        json.dump(out, fh, ensure_ascii=False, indent=1, sort_keys=True)
        fh.write("\n")
    print(f"generated {OUT}: {len(out)} keys, {count} entries (weight={WEIGHT})")
    return 0


if __name__ == "__main__":
    sys.exit(main())