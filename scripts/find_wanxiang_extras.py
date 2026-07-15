#!/usr/bin/env python3
"""
Find Wanxiang entries not in Qianyan, export as a new dictionary file.
Also converts the entire Wanxiang dict to Qianyan JSON format for reference.
"""

import json
import os

WANXIANG_DIR = '/tmp/rime-wanxiang/dicts'

TONE_CHARS = {
    '\u0101': 'a', '\u00e1': 'a', '\u01ce': 'a', '\u00e0': 'a',
    '\u0113': 'e', '\u00e9': 'e', '\u011b': 'e', '\u00e8': 'e',
    '\u012b': 'i', '\u00ed': 'i', '\u01d0': 'i', '\u00ec': 'i',
    '\u014d': 'o', '\u00f3': 'o', '\u01d2': 'o', '\u00f8': 'o',
    '\u0171': 'o', '\u00f2': 'o',
    '\u016b': 'u', '\u00fa': 'u', '\u01d4': 'u', '\u00f9': 'u',
    '\u01d6': 'v', '\u01d8': 'v', '\u01da': 'v', '\u01dc': 'v',
    '\u00fc': 'v',
}
_plain_table = str.maketrans(TONE_CHARS)

def to_plain(pinyin):
    return pinyin.lower().translate(_plain_table)


def load_qianyan_set():
    """Load all (key, word) pairs from existing Qianyan dictionaries."""
    qianyan = set()
    for root_dir in ['dicts/chinese/chars', 'dicts/chinese/words']:
        for fn in os.listdir(root_dir):
            if fn.endswith('.json') and fn != 'emoji_zh.json':
                fp = os.path.join(root_dir, fn)
                with open(fp, encoding='utf-8') as f:
                    data = json.load(f)
                for py_key, entries in data.items():
                    for entry in entries:
                        word = entry.get('char') or entry.get('word', '')
                        qianyan.add((py_key, word))
    return qianyan


def convert_wanxiang_dict(filepath):
    """Parse a Wanxiang .dict.yaml, yield (plain_key, word, weight, raw_pinyin)."""
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
                yield plain_key, word, weight, raw_pinyin


def main():
    print("Loading existing Qianyan dictionary entries...")
    qianyan = load_qianyan_set()
    print(f"  {len(qianyan)} entries in Qianyan")

    # Wanxiang word sources (all non-char dicts)
    word_files = [
        ('基础词库', 'jichu.dict.yaml'),
        ('联想词库', 'lianxiang.dict.yaml'),
        ('地名',     'diming.dict.yaml'),
        ('人名',     'renming.dict.yaml'),
        ('诗词',     'shici.dict.yaml'),
        ('化学',     'huaxue.dict.yaml'),
        ('物种',     'wuzhong.dict.yaml'),
        ('药品',     'yaopin.dict.yaml'),
        ('医学',     'yixue.dict.yaml'),
        ('艺人',     'yiren.dict.yaml'),
        ('名人',     'mingren.dict.yaml'),
        ('混编',     'mixed.dict.yaml'),
        ('错音',     'cuoyin.dict.yaml'),
        ('多音',     'duoyin.dict.yaml'),
    ]

    # Track seen to avoid duplicates across Wanxiang source files
    seen = set()
    extras = {}  # plain_key -> list of entries

    for label, fn in word_files:
        fp = os.path.join(WANXIANG_DIR, fn)
        if not os.path.exists(fp):
            continue

        count = 0
        new_count = 0
        for plain_key, word, weight, raw_pinyin in convert_wanxiang_dict(fp):
            count += 1
            # Skip single chars (already in char dict)
            if len(word) < 2:
                continue

            # Skip English entries where key == word (case-insensitive)
            if plain_key.lower() == word.lower():
                continue

            dedup_key = (plain_key, word)
            if dedup_key in seen:
                continue
            seen.add(dedup_key)

            # Check if already in Qianyan
            if dedup_key not in qianyan:
                entry = {
                    'char': word,
                    'weight': weight,
                    'tone': raw_pinyin,
                    'source': label,
                }
                extras.setdefault(plain_key, []).append(entry)
                new_count += 1

        print(f"  {label} ({fn}): {count} entries, {new_count} new to Qianyan")

    # Sort entries by weight desc within each key
    for py in extras:
        extras[py].sort(key=lambda e: -e['weight'])

    total_new = sum(len(v) for v in extras.values())
    print(f"\nTotal new Wanxiang entries not in Qianyan: {total_new}")
    print(f"Unique pinyin keys: {len(extras)}")

    # Write the extras
    out_path = 'dicts/chinese/words/wanxiang_extras.json'
    with open(out_path, 'w', encoding='utf-8') as f:
        json.dump(dict(sorted(extras.items())), f, ensure_ascii=False, indent=2)
    print(f"Written to {out_path}")

    # Stats
    total_wanxiang = len(seen)
    overlap = total_wanxiang - total_new
    print(f"\nWanxiang total words: {total_wanxiang}")
    print(f"Already in Qianyan: {overlap} ({overlap*100/total_wanxiang:.1f}%)")
    print(f"New extras: {total_new} ({total_new*100/total_wanxiang:.1f}%)")

    # Top entries by weight
    print("\nTop 20 new entries by weight:")
    all_entries = []
    for py, entries in extras.items():
        for e in entries:
            all_entries.append((e['weight'], py, e['char'], e['source']))
    all_entries.sort(reverse=True)
    for w, py, word, src in all_entries[:20]:
        print(f"  {word} ({py}) weight={w} [{src}]")


if __name__ == '__main__':
    main()
