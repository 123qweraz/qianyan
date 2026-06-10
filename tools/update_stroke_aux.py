import json

# ── SBSRF Matrix ──
MATRIX = {
    1: {1: 'g', 2: 'f', 3: 'd', 4: 's', 5: 'a'},
    2: {1: 'h', 2: 'j', 3: 'k', 4: 'l', 5: 'm'},
    3: {1: 't', 2: 'r', 3: 'e', 4: 'w', 5: 'q'},
    4: {1: 'y', 2: 'u', 3: 'i', 4: 'o', 5: 'p'},
    5: {1: 'n', 2: 'b', 3: 'v', 4: 'c', 5: 'x'},
}
SINGLE_MAP = {'1': 'g', '2': 'h', '3': 't', '4': 'y', '5': 'n'}


def compute_code(strokes, py_initial):
    s = [int(c) for c in strokes if c in "12345"]
    n = len(s)
    if n == 0:
        return f"x{py_initial}"
    if n == 1:
        first = SINGLE_MAP.get(strokes[0], 'x')
        last = first
    else:
        first = MATRIX[s[0]][s[1]]
        last = MATRIX[s[-2]][s[-1]]
    return f"{first}{last}{py_initial}"


def main():
    import sys
    if len(sys.argv) < 2:
        print("Usage: python3 update_stroke_aux.py <level.json> [bihua.txt]")
        return

    level_path = sys.argv[1]
    bihua_path = sys.argv[2] if len(sys.argv) > 2 else "tools/bihua.txt"

    # Load stroke data
    stroke_map = {}
    with open(bihua_path) as f:
        for line in f:
            line = line.strip()
            if '\t' in line:
                ch, strokes = line.split('\t', 1)
                stroke_map[ch] = strokes.strip()
    print(f"Loaded {len(stroke_map)} characters from {bihua_path}")

    # Load level dict
    with open(level_path) as f:
        data = json.load(f)

    updated = 0
    total = 0
    for pinyin_key, entries in data.items():
        py_init = pinyin_key[0].lower() if pinyin_key else 'x'
        for entry in entries:
            total += 1
            ch = entry['char']
            if ch in stroke_map:
                strokes = stroke_map[ch]
                entry['stroke_aux'] = compute_code(strokes, py_init)
                updated += 1

    with open(level_path, 'w', encoding='utf-8') as f:
        json.dump(data, f, ensure_ascii=False, indent=2)

    print(f"Total entries: {total}")
    print(f"Updated: {updated}")
    print(f"Skipped (no stroke data): {total - updated}")


if __name__ == "__main__":
    main()
