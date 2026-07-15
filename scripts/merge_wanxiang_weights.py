#!/usr/bin/env python3
"""
Merge Wanxiang dictionary weights into existing Qianyan dictionary files.
- For entries found in Wanxiang: update weight to Wanxiang's value
- For 多音字 (char exists in Wanxiang under diff pinyin): use that weight
- For entries not in Wanxiang at all: keep original weight
"""

import json
import os
from collections import defaultdict

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


def load_wanxiang_weights(filepath):
    """Load Wanxiang dict file into:
       - {(plain_key, word): weight}  (exact match)
       - {word: max_weight}           (word exists under any pinyin)
    """
    exact = {}
    word_max = {}
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
                key = (plain_key, word)
                if key not in exact or weight > exact[key]:
                    exact[key] = weight
                if word not in word_max or weight > word_max[word]:
                    word_max[word] = weight
    return exact, word_max


def merge_weights(json_path, wanxiang_exact, wanxiang_word_max, dict_name, is_char=True):
    """Merge weights: Wanxiang > 多音字复用 > 保留原值"""
    with open(json_path, encoding='utf-8') as f:
        data = json.load(f)

    updated_wanxiang = 0
    updated_poly = 0
    kept_original = 0
    total = 0

    for pinyin_key, entries in data.items():
        for entry in entries:
            total += 1
            word = entry.get('char') or entry.get('word', '')
            key = (pinyin_key, word)
            old_w = entry.get('weight', 0)

            if key in wanxiang_exact:
                new_w = wanxiang_exact[key]
                if old_w != new_w:
                    entry['weight'] = new_w
                    updated_wanxiang += 1
            elif word in wanxiang_word_max:
                new_w = wanxiang_word_max[word]
                if old_w != new_w:
                    entry['weight'] = new_w
                    updated_poly += 1
            else:
                kept_original += 1

    with open(json_path, 'w', encoding='utf-8') as f:
        json.dump(data, f, ensure_ascii=False, indent=2)

    print(f"  {dict_name}: {total} entries | 万象更新={updated_wanxiang} 多音字复用={updated_poly} 保留原值={kept_original}")
    return updated_wanxiang + updated_poly


def main():
    # ── Load all Wanxiang data ──
    wanxiang_exact = {}
    wanxiang_word_max = {}

    # Chars
    e, w = load_wanxiang_weights(os.path.join(WANXIANG_DIR, 'zi.dict.yaml'))
    wanxiang_exact.update(e)
    wanxiang_word_max.update(w)
    print(f"Loaded {len(wanxiang_exact)} exact + {len(wanxiang_word_max)} unique words from zi.dict.yaml")

    # Words
    word_files = [
        'jichu.dict.yaml', 'lianxiang.dict.yaml', 'diming.dict.yaml',
        'renming.dict.yaml', 'shici.dict.yaml', 'huaxue.dict.yaml',
        'wuzhong.dict.yaml', 'yaopin.dict.yaml', 'yixue.dict.yaml',
        'yiren.dict.yaml', 'mingren.dict.yaml', 'mixed.dict.yaml',
        'cuoyin.dict.yaml', 'duoyin.dict.yaml', 'en.dict.yaml',
    ]
    for fn in word_files:
        fp = os.path.join(WANXIANG_DIR, fn)
        if os.path.exists(fp):
            e, w = load_wanxiang_weights(fp)
            wanxiang_exact.update(e)
            for word, weight in w.items():
                if word not in wanxiang_word_max or weight > wanxiang_word_max[word]:
                    wanxiang_word_max[word] = weight
    print(f"Loaded {len(wanxiang_exact)} total exact entries from Wanxiang")

    total_updated = 0

    # ── Char files ──
    for fn in sorted(os.listdir('dicts/chinese/chars')):
        if fn.endswith('.json'):
            fp = os.path.join('dicts/chinese/chars', fn)
            total_updated += merge_weights(fp, wanxiang_exact, wanxiang_word_max, f"chars/{fn}", is_char=True)

    # ── Word files ──
    for fn in sorted(os.listdir('dicts/chinese/words')):
        if fn.endswith('.json') and fn != 'emoji_zh.json':
            fp = os.path.join('dicts/chinese/words', fn)
            total_updated += merge_weights(fp, wanxiang_exact, wanxiang_word_max, f"words/{fn}", is_char=False)

    print(f"\nTotal weight changes: {total_updated}")


if __name__ == '__main__':
    main()
