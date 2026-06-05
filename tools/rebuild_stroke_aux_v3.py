import json
import os
from pathlib import Path

# 笔画矩阵 (SBSRF)
MATRIX = {
    1: {1: 'g', 2: 'f', 3: 'd', 4: 's', 5: 'a'},
    2: {1: 'h', 2: 'j', 3: 'k', 4: 'l', 5: 'm'},
    3: {1: 't', 2: 'r', 3: 'e', 4: 'w', 5: 'q'},
    4: {1: 'y', 2: 'u', 3: 'i', 4: 'o', 5: 'p'},
    5: {1: 'n', 2: 'b', 3: 'v', 4: 'c', 5: 'x'}
}
SINGLE_MAP = {'1': 'g', '2': 'h', '3': 't', '4': 'y', '5': 'n'}

def encode_pair(s1, s2):
    return MATRIX[s1][s2]

def encode_single(s):
    return SINGLE_MAP.get(s, 'x')

def get_py_init_from_tone(tone):
    if not tone:
        return 'x'
    c = tone[0].lower()
    return c if 'a' <= c <= 'z' else 'x'

def compute_code(strokes, py_initial):
    s = [int(c) for c in strokes if c in "12345"]
    n = len(s)
    if n == 0:
        return f"x{py_initial}"
    if n == 1:
        first = encode_single(strokes[0])
        last = first
    else:
        first = encode_pair(s[0], s[1])
        last = encode_pair(s[-2], s[-1])
    return f"{first}{last}{py_initial}"

def is_compact_json(path):
    with open(path, 'rb') as f:
        raw = f.read(200)
    return b'\n' not in raw.strip()

def update_level_dict(path):
    if not path.exists():
        print(f"  SKIP (not found): {path}")
        return
    print(f"  Updating: {path.name}")
    data = json.loads(path.read_text(encoding='utf-8'))
    updated = 0
    for pinyin_key, entries in data.items():
        py_init = pinyin_key[0].lower() if pinyin_key else 'x'
        for entry in entries:
            strokes = entry.get('strokes', '')
            if strokes:
                entry['stroke_aux'] = compute_code(strokes, py_init)
                updated += 1
    path.write_text(json.dumps(data, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    print(f"    Updated {updated} entries")

def rekey_stroke_chars(path, level_filter=None):
    if not path.exists():
        print(f"  SKIP (not found): {path}")
        return
    compact = is_compact_json(path)
    print(f"  Rekeying: {path.name} (compact={compact})")
    data = json.loads(path.read_text(encoding='utf-8'))
    new_data = {}
    for old_key, entries in data.items():
        for entry in entries:
            cat = entry.get('category', '')
            if level_filter and cat not in level_filter:
                continue
            strokes = entry.get('strokes', '')
            tone = entry.get('tone', '')
            if not strokes:
                continue
            new_key = compute_code(strokes, get_py_init_from_tone(tone))
            if new_key not in new_data:
                new_data[new_key] = []
            new_data[new_key].append(entry)
    if compact:
        text = json.dumps(new_data, ensure_ascii=False, separators=(',', ':'))
    else:
        text = json.dumps(new_data, ensure_ascii=False, indent=2) + '\n'
    path.write_text(text, encoding='utf-8')
    print(f"    Keys: {len(data)} -> {len(new_data)}, Entries: {sum(len(v) for v in data.values())}")

def rebuild_stroke_map(src_path, dst_path):
    print(f"  Building stroke_map.txt from {src_path.name}")
    data = json.loads(src_path.read_text(encoding='utf-8'))
    lines = []
    for code, entries in data.items():
        for entry in entries:
            char = entry.get('char', '')
            if char:
                lines.append(f"{char} {code}")
    lines.sort()
    dst_path.write_text('\n'.join(lines) + '\n', encoding='utf-8')
    print(f"    Written {len(lines)} entries")

def main():
    root = Path(__file__).resolve().parent.parent
    chars_dir = root / "dicts" / "chinese" / "chars"
    sheng_dir = root / "dicts" / "shengpizi"
    stroke_dir = root / "dicts" / "stroke"

    print("=" * 60)
    print("Rebuilding stroke_aux with NEW 3-code rule")
    print("  Pos 1 = first 2 strokes (pair code)")
    print("  Pos 2 = last 2 strokes (pair code)")
    print("  Pos 3 = pinyin initial")
    print("=" * 60)

    print("\n[1/5] Updating level JSON dictionaries (stroke_aux field)...")
    for name in ["level1.json", "level2.json", "level3.json"]:
        update_level_dict(chars_dir / name)

    print("\n[2/5] Rekeying shengpizi/stroke_chars.json (master)...")
    rekey_stroke_chars(sheng_dir / "stroke_chars.json")

    print("\n[3/5] Rebuilding dicts/stroke/chars/stroke_chars.json (compact, levels 1-3)...")
    src = sheng_dir / "stroke_chars.json"
    dst = stroke_dir / "chars" / "stroke_chars.json"
    dst.write_text(
        json.dumps(
            {k: v for k, v in json.loads(src.read_text('utf-8')).items()
             if any(e.get('category') in ('level-1','level-2','level-3') for e in v)},
            ensure_ascii=False, separators=(',', ':')
        ),
        encoding='utf-8'
    )
    print(f"    Written {dst.name}")

    print("\n[4/5] Rekeying stroke_chars_level-*.json...")
    for suffix in ["level-1", "level-2", "level-3", "level-4"]:
        rekey_stroke_chars(stroke_dir / "chars" / f"stroke_chars_{suffix}.json")

    print("\n[5/5] Regenerating stroke_map.txt...")
    rebuild_stroke_map(sheng_dir / "stroke_chars.json", stroke_dir / "stroke_map.txt")

    print("\n" + "=" * 60)
    print("DONE!")
    print("=" * 60)

if __name__ == "__main__":
    main()
