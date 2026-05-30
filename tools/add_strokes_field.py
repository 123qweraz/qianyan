import json
import os

def load_bihua(file_path):
    char_to_strokes = {}
    if not os.path.exists(file_path):
        print(f"Bihua file not found: {file_path}")
        return char_to_strokes
    
    with open(file_path, 'r', encoding='utf-8') as f:
        for line in f:
            parts = line.strip().split()
            if len(parts) >= 2:
                char = parts[0]
                strokes = parts[1]
                char_to_strokes[char] = strokes
    return char_to_strokes

def update_dict(file_path, char_to_strokes):
    if not os.path.exists(file_path):
        print(f"Dictionary file not found: {file_path}")
        return

    with open(file_path, 'r', encoding='utf-8') as f:
        data = json.load(f)
    
    updated_count = 0
    total_chars = 0
    missing_chars = []

    for pinyin, items in data.items():
        for item in items:
            total_chars += 1
            char = item.get('char')
            if char in char_to_strokes:
                item['strokes'] = char_to_strokes[char]
                updated_count += 1
            else:
                missing_chars.append(f"{char}({pinyin})")
    
    with open(file_path, 'w', encoding='utf-8') as f:
        json.dump(data, f, ensure_ascii=False, indent=2)
    
    print(f"Updated {updated_count}/{total_chars} characters in {file_path}")
    if missing_chars:
        print(f"Missing strokes for {len(missing_chars)} chars. Samples: {', '.join(missing_chars[:10])}")

if __name__ == "__main__":
    bihua_map = load_bihua('dicts/chinese/chars/bihua.txt')
    print(f"Loaded {len(bihua_map)} characters from bihua.txt")
    
    dicts = [
        'dicts/chinese/chars/chars.json',
        'dicts/chinese/chars/level2.json',
        'dicts/chinese/chars/level3.json'
    ]
    
    for d in dicts:
        update_dict(d, bihua_map)
