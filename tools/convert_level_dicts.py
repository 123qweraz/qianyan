import json
import os
from pypinyin import pinyin, Style

def convert_file(input_path, output_path, category_label):
    if not os.path.exists(input_path):
        print(f"File not found: {input_path}")
        return

    print(f"Converting {input_path} -> {output_path}...")
    with open(input_path, 'r', encoding='utf-8') as f:
        data = json.load(f)

    # New structure: { "pinyin": [ { "char": "...", "en": "...", "category": "...", "tone": "..." }, ... ] }
    new_data = {}

    for entry in data:
        char = entry.get('char')
        if not char:
            continue
        
        # Get pinyin without tone marks for keys
        py_list = pinyin(char, style=Style.NORMAL, heteronym=True)[0]
        # Get pinyin with tone marks for hints
        tone_list = pinyin(char, style=Style.TONE, heteronym=True)[0]
        
        # Handle polyphones (multiple pronunciations)
        for i, py in enumerate(py_list):
            if py not in new_data:
                new_data[py] = []
            
            tone_mark = tone_list[i] if i < len(tone_list) else py
            
            new_entry = {
                "char": char,
                "en": entry.get('en', ''),
                "category": category_label,
                "tone": f"{py}/{tone_mark}"
            }
            new_data[py].append(new_entry)

    # Sort keys for cleaner JSON
    sorted_data = {k: new_data[k] for k in sorted(new_data.keys())}

    with open(output_path, 'w', encoding='utf-8') as f:
        json.dump(sorted_data, f, ensure_ascii=False, indent=2)
    print(f"Done. Entries: {len(data)}, Pinyin keys: {len(sorted_data)}")

if __name__ == "__main__":
    convert_file('dicts/chinese/level2_raw.json', 'dicts/chinese/level2.json', 'level-2')
    convert_file('dicts/chinese/level3_raw.json', 'dicts/chinese/level3.json', 'level-3')
