#!/usr/bin/env python3
"""从 words.json / new_words.json 统计 2~4 字拼音的频率（weight 总和）"""
import json
import os
from collections import defaultdict

DICT_DIR = "dicts/chinese/words"
OUTPUT = "dicts/chinese/syllable_freq.txt"

freq: dict[str, int] = defaultdict(int)

for fname in ["words.json", "new_words.json"]:
    path = os.path.join(DICT_DIR, fname)
    if not os.path.exists(path):
        continue
    with open(path, encoding="utf-8") as f:
        data = json.load(f)
    for pinyin, entries in data.items():
        total_weight = 0
        has_valid = False
        for entry in entries:
            if not isinstance(entry, dict):
                continue
            char = entry.get("char") or entry.get("word") or ""
            weight = entry.get("weight") or 0
            cjk_count = sum(1 for c in char if '\u4e00' <= c <= '\u9fff')
            if 2 <= cjk_count <= 4:
                total_weight += weight
                has_valid = True
        if has_valid:
            freq[pinyin] += total_weight

with open(OUTPUT, "w", encoding="utf-8") as f:
    for pinyin in sorted(freq):
        f.write(f"{pinyin} {freq[pinyin]}\n")

print(f"Generated {OUTPUT}: {len(freq)} entries")
