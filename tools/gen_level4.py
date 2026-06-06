#!/usr/bin/env python3
"""Generate level4.json: characters from bihua.txt not in level1-3.json.

Uses Unihan_Readings.txt (kMandarin field) for pinyin data of rare/CJK extension characters.
"""
import json
import re
from pathlib import Path

# ── SBSRF Stroke Auxiliary Code Matrix ──
MATRIX = {
    1: {1: 'g', 2: 'f', 3: 'd', 4: 's', 5: 'a'},
    2: {1: 'h', 2: 'j', 3: 'k', 4: 'l', 5: 'm'},
    3: {1: 't', 2: 'r', 3: 'e', 4: 'w', 5: 'q'},
    4: {1: 'y', 2: 'u', 3: 'i', 4: 'o', 5: 'p'},
    5: {1: 'n', 2: 'b', 3: 'v', 4: 'c', 5: 'x'},
}
SINGLE_MAP = {'1': 'g', '2': 'h', '3': 't', '4': 'y', '5': 'n'}

# Tone mark → plain vowel mapping
TONE_STRIP = str.maketrans({
    'ā': 'a', 'á': 'a', 'ǎ': 'a', 'à': 'a',
    'ē': 'e', 'é': 'e', 'ě': 'e', 'è': 'e',
    'ī': 'i', 'í': 'i', 'ǐ': 'i', 'ì': 'i',
    'ō': 'o', 'ó': 'o', 'ǒ': 'o', 'ò': 'o',
    'ū': 'u', 'ú': 'u', 'ǔ': 'u', 'ù': 'u',
    'ǖ': 'ü', 'ǘ': 'ü', 'ǚ': 'ü', 'ǜ': 'ü',
})


def strip_tone(py):
    """Strip tone marks from pinyin (e.g. 'yī' → 'yi')."""
    return py.translate(TONE_STRIP)


def compute_stroke_aux(strokes, py_initial):
    """Compute SBSRF 3-code stroke auxiliary code."""
    s = [int(c) for c in strokes if c in "12345"]
    n = len(s)
    if n == 0:
        return f"x{py_initial}x" if py_initial else "xxx"
    if n == 1:
        code = SINGLE_MAP.get(strokes[0], 'x')
        return f"{code}{code}{py_initial}" if py_initial else f"{code}{code}x"
    first = MATRIX[s[0]][s[1]]
    last = MATRIX[s[-2]][s[-1]]
    return f"{first}{last}{py_initial}" if py_initial else f"{first}{last}x"


def load_single_syllables(chars_dir):
    """Load all valid single pinyin syllables from level1-3.json keys."""
    syllables = set()
    for name in ["level1.json", "level2.json", "level3.json"]:
        path = chars_dir / name
        if path.exists():
            data = json.loads(path.read_text(encoding='utf-8'))
            syllables.update(data.keys())
    return syllables


def load_existing_chars(chars_dir):
    """Load all characters already in level1-3.json."""
    chars = set()
    for name in ["level1.json", "level2.json", "level3.json"]:
        path = chars_dir / name
        if not path.exists():
            continue
        data = json.loads(path.read_text(encoding='utf-8'))
        for pinyin_key, entries in data.items():
            for entry in entries:
                ch = entry.get('char', '')
                if ch:
                    chars.add(ch)
    return chars


def load_bihua(bihua_path):
    """Load bihua.txt: mapping from char → strokes."""
    mapping = {}
    with open(bihua_path, encoding='utf-8') as f:
        for line in f:
            line = line.strip()
            if '\t' in line:
                ch, strokes = line.split('\t', 1)
                mapping[ch] = strokes.strip()
    return mapping


def parse_unihan_kmandarin(unihan_path):
    """Parse Unihan_Readings.txt kMandarin field → char → (pinyin_list, tone_str)."""
    result = {}
    with open(unihan_path, encoding='utf-8') as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith('#'):
                continue
            if 'kMandarin' not in line:
                continue
            parts = line.split('\t')
            if len(parts) < 3:
                continue
            code_hex = parts[0].replace('U+', '')
            try:
                ch = chr(int(code_hex, 16))
            except ValueError:
                continue
            readings = parts[2]
            # readings is like "yī, yí, yì" or "dīng, zhēng"
            # Split into individual readings
            reading_list = [r.strip() for r in re.split(r'[,; ]\s*', readings) if r.strip()]
            if not reading_list:
                continue
            # First reading as primary pinyin (strip tone)
            primary = strip_tone(reading_list[0])
            # Normalize ü → u for pinyin key matching
            primary = primary.replace('ü', 'u')
            result[ch] = {
                'pinyin': primary,
                'tone': '/'.join(reading_list),
            }
    return result


def main():
    root = Path(__file__).resolve().parent.parent
    chars_dir = root / "dicts" / "chinese" / "chars"
    bihua_path = root / "dicts" / "chinese" / "bihua.txt"
    unihan_path = Path("/tmp/Unihan_Readings.txt")
    output_path = chars_dir / "level4.json"

    if not unihan_path.exists():
        print("Error: Unihan_Readings.txt not found. Download it first.")
        print("Run: python3 -c \"import urllib.request, zipfile, io; "
              "req = urllib.request.Request('https://www.unicode.org/Public/UCD/latest/ucd/Unihan.zip', "
              "headers={'User-Agent': 'Mozilla/5.0'}); "
              "z = zipfile.ZipFile(io.BytesIO(urllib.request.urlopen(req, timeout=60).read())); "
              "z.extract('Unihan_Readings.txt', '/tmp/')\"")
        return

    print("Loading single syllables...")
    syllables = load_single_syllables(chars_dir)
    print(f"  {len(syllables)} syllables")

    print("Loading existing chars from level1-3.json...")
    existing = load_existing_chars(chars_dir)
    print(f"  {len(existing)} chars")

    print("Loading bihua.txt...")
    bihua = load_bihua(bihua_path)
    print(f"  {len(bihua)} entries")

    print("Parsing Unihan kMandarin...")
    char_pinyin = parse_unihan_kmandarin(unihan_path)
    print(f"  {len(char_pinyin)} chars with kMandarin pinyin")

    # Find missing chars
    missing_chars = [ch for ch in bihua if ch not in existing]
    print(f"  {len(missing_chars)} missing (in bihua, not in level1-3)")

    # Build level4 data
    level4 = {}
    skipped_no_pinyin = 0
    produced = 0

    for ch in missing_chars:
        strokes = bihua[ch]
        info = char_pinyin.get(ch)

        if not info:
            skipped_no_pinyin += 1
            continue

        pinyin_key = info['pinyin']
        tone = info['tone']
        py_init = pinyin_key[0].lower() if pinyin_key else 'x'

        stroke_aux = compute_stroke_aux(strokes, py_init)

        entry = {
            "category": "level-4",
            "char": ch,
            "stroke_aux": stroke_aux,
            "tone": tone,
            "trad": ch,
            "weight": 0,
            "strokes": strokes,
        }

        if pinyin_key not in level4:
            level4[pinyin_key] = []
        level4[pinyin_key].append(entry)
        produced += 1

    # Write level4.json
    output_path.write_text(
        json.dumps(level4, ensure_ascii=False, indent=2) + '\n',
        encoding='utf-8'
    )

    total_entries = sum(len(v) for v in level4.values())
    print(f"\nDone! Generated {output_path}")
    print(f"  Pinyin keys: {len(level4)}")
    print(f"  Total entries: {total_entries}")
    print(f"  Skipped (no pinyin): {skipped_no_pinyin}")

    # Stats
    chars_with_pinyin = len(existing) + produced
    total_in_bihua = len(bihua)
    print(f"  Coverage in bihua.txt: {chars_with_pinyin}/{total_in_bihua} "
          f"({100 * chars_with_pinyin / total_in_bihua:.1f}%)")

    # Show samples
    print("\nSample entries (first 10):")
    shown = 0
    for pinyin_key, entries in sorted(level4.items()):
        for entry in entries[:2]:
            print(f"  [{pinyin_key}] {entry['char']} U+{ord(entry['char']):04X} "
                  f"strokes={entry['strokes']} aux={entry['stroke_aux']} tone={entry['tone']}")
            shown += 1
            if shown >= 10:
                break
        if shown >= 10:
            break


if __name__ == "__main__":
    main()
