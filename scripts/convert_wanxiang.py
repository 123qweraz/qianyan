#!/usr/bin/env python3
"""
Convert Wanxiang RIME dictionaries to Qianyan IME JSON format.

Usage: python3 scripts/convert_wanxiang.py
"""

import json
import os
from collections import defaultdict

WANXIANG_DIR = '/tmp/rime-wanxiang/dicts'
OUT_CHARS_DIR = 'dicts/chinese/chars'
OUT_WORDS_DIR = 'dicts/chinese/words'

TONE_CHARS = {
    '\u0101': ('a', '1'), '\u00e1': ('a', '2'), '\u01ce': ('a', '3'), '\u00e0': ('a', '4'),
    '\u0113': ('e', '1'), '\u00e9': ('e', '2'), '\u011b': ('e', '3'), '\u00e8': ('e', '4'),
    '\u012b': ('i', '1'), '\u00ed': ('i', '2'), '\u01d0': ('i', '3'), '\u00ec': ('i', '4'),
    '\u014d': ('o', '1'), '\u00f3': ('o', '2'), '\u01d2': ('o', '3'), '\u00f2': ('o', '4'),
    '\u016b': ('u', '1'), '\u00fa': ('u', '2'), '\u01d4': ('u', '3'), '\u00f9': ('u', '4'),
    '\u01d6': ('v', '1'), '\u01d8': ('v', '2'), '\u01da': ('v', '3'), '\u01dc': ('v', '4'),
    '\u00fc': ('v', '5'),
}

_plain_table = str.maketrans({k: v[0] for k, v in TONE_CHARS.items()})

def to_plain(pinyin):
    return pinyin.lower().translate(_plain_table)


def parse_dict_file(filepath):
    with open(filepath, encoding='utf-8') as f:
        in_data = False
        for line in f:
            line = line.rstrip('\n\r')
            if not in_data:
                if line.strip() == '...':
                    in_data = True
                continue
            if not line or line.startswith('#'):
                continue
            parts = line.split('\t')
            if len(parts) >= 3:
                word = parts[0]
                raw_pinyin = parts[1]
                try:
                    weight = int(parts[2])
                except ValueError:
                    continue
                combined = raw_pinyin.replace(' ', '')
                plain_key = to_plain(combined)
                yield word, raw_pinyin, plain_key, weight


def convert_chars():
    print("Converting characters...")
    filepath = os.path.join(WANXIANG_DIR, 'zi.dict.yaml')

    char_entries = {}
    tone_map = defaultdict(set)

    for word, raw_pinyin, plain_key, weight in parse_dict_file(filepath):
        if len(word) != 1:
            continue
        key = (plain_key, word)
        if key not in char_entries or weight > char_entries[key]:
            char_entries[key] = weight
        tone_map[key].add(raw_pinyin)

    output = defaultdict(list)
    for (plain_key, word), weight in sorted(char_entries.items()):
        tones = sorted(tone_map[(plain_key, word)])
        tone_display = '/'.join(tones)

        entry = {'char': word, 'weight': weight, 'tone': tone_display}
        output[plain_key].append(entry)

    out_path = os.path.join(OUT_CHARS_DIR, 'level1.json')
    with open(out_path, 'w', encoding='utf-8') as f:
        json.dump(dict(sorted(output.items())), f, ensure_ascii=False, indent=2)

    total = sum(len(v) for v in output.values())
    print(f"  Wrote {total} chars to {out_path}")
    print(f"  Unique pinyin keys: {len(output)}")


def convert_words():
    print("Converting words...")

    WORD_SOURCES = [
        'jichu.dict.yaml',
        'lianxiang.dict.yaml',
        'diming.dict.yaml',
        'renming.dict.yaml',
        'shici.dict.yaml',
        'huaxue.dict.yaml',
        'wuzhong.dict.yaml',
        'yaopin.dict.yaml',
        'yixue.dict.yaml',
        'yiren.dict.yaml',
        'mingren.dict.yaml',
        'cuoyin.dict.yaml',
        'duoyin.dict.yaml',
        'en.dict.yaml',
    ]

    all_entries = defaultdict(list)
    seen = set()

    for filename in WORD_SOURCES:
        filepath = os.path.join(WANXIANG_DIR, filename)
        if not os.path.exists(filepath):
            continue

        count = 0
        for word, raw_pinyin, plain_key, weight in parse_dict_file(filepath):
            if len(word) < 2:
                continue

            dedup_key = (plain_key, word)
            if dedup_key in seen:
                continue
            seen.add(dedup_key)

            entry = {'char': word, 'weight': weight, 'tone': raw_pinyin}
            all_entries[plain_key].append(entry)
            count += 1

        print(f"  Parsed {filename}: {count} entries")

    high_freq = {}
    low_freq = {}

    for pinyin_key, entries in all_entries.items():
        entries.sort(key=lambda e: -e['weight'])
        high_list = [e for e in entries if e['weight'] >= 100]
        low_list = [e for e in entries if e['weight'] < 100]
        if high_list:
            high_freq[pinyin_key] = high_list
        if low_list:
            low_freq[pinyin_key] = low_list

    high_path = os.path.join(OUT_WORDS_DIR, 'high_freq.json')
    with open(high_path, 'w', encoding='utf-8') as f:
        json.dump(dict(sorted(high_freq.items())), f, ensure_ascii=False, indent=2)
    print(f"  Wrote {sum(len(v) for v in high_freq.values())} words to {high_path}")

    low_path = os.path.join(OUT_WORDS_DIR, 'low_freq.json')
    with open(low_path, 'w', encoding='utf-8') as f:
        json.dump(dict(sorted(low_freq.items())), f, ensure_ascii=False, indent=2)
    print(f"  Wrote {sum(len(v) for v in low_freq.values())} words to {low_path}")


def verify():
    print("\n=== Verification ===")
    with open(os.path.join(OUT_CHARS_DIR, 'level1.json')) as f:
        chars = json.load(f)

    print("\nTop char entries for 'bian':")
    for e in sorted(chars.get('bian', []), key=lambda x: -x['weight'])[:5]:
        print(f"  {e['char']}: weight={e['weight']}")

    print("\nTop char entries for 'lian':")
    for e in sorted(chars.get('lian', []), key=lambda x: -x['weight'])[:5]:
        print(f"  {e['char']}: weight={e['weight']}")

    with open(os.path.join(OUT_WORDS_DIR, 'high_freq.json')) as f:
        words = json.load(f)

    print("\nWord entries for pinyin 'bian':")
    for e in words.get('bian', [])[:5]:
        print(f"  {e['char']}: weight={e['weight']}")

    print("\nWord entries for pinyin 'lian':")
    for e in words.get('lian', [])[:5]:
        print(f"  {e['char']}: weight={e['weight']}")

    # Verify no 彼岸 with weight > 变
    for py, entries in words.items():
        for e in entries:
            if e['char'] == '彼岸':
                print(f"\n彼岸 found: pinyin='{py}', weight={e['weight']}")
            if e['char'] == '立案':
                print(f"立案 found: pinyin='{py}', weight={e['weight']}")


if __name__ == '__main__':
    os.makedirs(OUT_CHARS_DIR, exist_ok=True)
    os.makedirs(OUT_WORDS_DIR, exist_ok=True)

    convert_chars()
    convert_words()
    verify()

    print("\nDone!")
