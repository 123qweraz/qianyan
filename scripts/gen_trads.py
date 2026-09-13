#!/usr/bin/env python3
"""从 dicts/chinese/chars/ 生成繁体单字词条 trads.json（放在 chars/ 内一并编译）。

背景：繁体字大多以 level-4/5（生僻）形式存在，默认 CharMode 的过滤会把生僻
字挡掉，导致打拼音出不了繁体。本脚本把每个繁体单字以 category=trad 的普通候选
注入（编译为 FLAG_TRAD=0x02），使简体读音即可顺带打出繁体字，无须运行时简繁
转换；用户可通过设置中的"字符显示范围"(CharMode) 自行选择是否显示繁体字。

条目格式：
- char: 繁体单字本身（不设 trad 字段——它没有独立的繁体形式）
- weight: 简体原字权重 − 1（与原字同层可及、又恒在原字之后）；每键仅前 3 个
  最高权繁体进入高权重区，其余 weight=1 垫底（避免挤占正常词位）
- category: trad
- en: 拷贝简体原字的英文注释
- tone/stroke_aux: 若繁体以生僻元数据存在，拷贝其拼音声调/笔画码作提示

注：输入扫描会跳过本文件自身（否则二次生成会自包含而全部跳过）。
"""
import glob
import json
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CHARS_DIR = os.path.join(ROOT, "dicts", "chinese", "chars")
OUT = os.path.join(CHARS_DIR, "trads.json")
RARE_CATEGORIES = {"level-4", "level-5"}
CATEGORY = "trad"
SELF_NAME = os.path.basename(OUT)
FLOOR_WEIGHT = 1
HIGH_WEIGHT_PER_KEY = 3


def main() -> int:
    non_rare: set[tuple[str, str]] = set()
    rare_meta: dict[tuple[str, str], dict] = {}
    src: dict[tuple[str, str], tuple[int, str]] = {}
    trads: dict[str, set[str]] = {}

    for f in sorted(glob.glob(os.path.join(CHARS_DIR, "*.json"))):
        if os.path.basename(f) == SELF_NAME:
            continue
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
                if not (isinstance(trad, str) and len(trad) == 1 and trad != char):
                    continue
                trads.setdefault(key, set()).add(trad)
                wt = entry.get("weight", 0)
                w = int(wt) if isinstance(wt, (int, float)) else 0
                en = entry.get("en", "")
                if not isinstance(en, str):
                    en = ""
                old = src.get((key, trad))
                if old is None or w > old[0]:
                    src[(key, trad)] = (w, en)

    out: dict[str, list[dict]] = {}
    count = 0
    high_count = 0
    for key in sorted(trads):
        pairs = [
            (t, src.get((key, t), (0, ""))[0], src.get((key, t), (0, ""))[1])
            for t in trads[key]
            if (key, t) not in non_rare
        ]
        pairs.sort(key=lambda p: (-p[1], p[0]))
        for idx, (trad, w, en) in enumerate(pairs):
            meta = rare_meta.get((key, trad))
            if idx < HIGH_WEIGHT_PER_KEY and w > 0:
                weight = max(FLOOR_WEIGHT, w - 1)
                high_count += 1
            else:
                weight = FLOOR_WEIGHT
            entry: dict = {
                "char": trad,
                "weight": weight,
                "category": CATEGORY,
            }
            if en:
                entry["en"] = en
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
    print(
        f"generated {OUT}: {len(out)} keys, {count} entries "
        f"(high_weight={high_count}, floor_weight={FLOOR_WEIGHT}, per_key_high={HIGH_WEIGHT_PER_KEY})"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())