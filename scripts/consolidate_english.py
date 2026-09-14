#!/usr/bin/env python3
"""合并 dicts/english/*.json 为一本规范词库 combined_dict.json。

背景：english 方案下有多种文件格式并存——
- string-array 格式（combined_dict/dict_5_10s/dict_enlt5s/linux_commands）：key 即英文词，数组为中文词注
- object 格式（english_dict_/english_dict_geater5）：数组元素为 {"char":词, "en":词注,...}
编译期去重先到先得，且 object 格式的词注依赖编译顺序才能存活，导致多头怪象。
这里把全部来源合成一本统一 string-array 格式的 combined_dict.json，剔除依赖顺序的隐含行为。

词注优先取 string-array 来源（按优先级排列），object 来源仅兜底。
输出 JSON 键与数组均排序，保证可重复 diff。
用法：python3 scripts/consolidate_english.py
"""
import json
import os
from collections import OrderedDict

D = os.path.join(os.path.dirname(__file__), "..", "dicts", "english")

STRING_FILES = ["combined_dict.json", "dict_5_10s.json", "dict_enlt5s.json", "linux_commands.json"]
OBJECT_FILES = ["english_dict_.json", "english_dict_geater5.json"]
# object 来源的导出路径（合并后保留词，不再作为独立文件）
KEEP_STRING = ["combined_dict.json"]
DEAD_FILES = ["en_conflict_suggestions.json", "merge_dicts.py"]


def load(fname):
    with open(os.path.join(D, fname), encoding="utf-8") as f:
        return json.load(f)


merged = OrderedDict()  # key -> [gloss]

for fname in STRING_FILES:
    for key, glosses in load(fname).items():
        merged.setdefault(key, [])
        for g in glosses:
            if g and g not in merged[key]:
                merged[key].append(g)

for fname in OBJECT_FILES:
    for key, objs in load(fname).items():
        for o in objs:
            gloss = o.get("en") if isinstance(o, dict) else o
            if gloss and key not in merged:
                merged[key] = [gloss]

# 排序保证确定性输出
merged = OrderedDict(sorted(merged.items()))
for k in merged:
    merged[k].sort()

out = os.path.join(D, "combined_dict.json")
with open(out, "w", encoding="utf-8") as f:
    json.dump(merged, f, ensure_ascii=False, indent=2)
    f.write("\n")

for fname in DEAD_FILES + OBJECT_FILES:
    p = os.path.join(D, fname)
    if os.path.exists(p):
        os.remove(p)

print(f"merged {len(merged)} keys -> {out}")