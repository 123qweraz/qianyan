import json
import os

def load_map(file_path):
    char_map = {}
    with open(file_path, 'r', encoding='utf-8') as f:
        for line in f:
            parts = line.strip().split(' ')
            if len(parts) == 2:
                char_map[parts[0]] = parts[1]
    return char_map

def update_dict(file_path, char_map):
    if not os.path.exists(file_path):
        print(f"File not found: {file_path}")
        return

    with open(file_path, 'r', encoding='utf-8') as f:
        data = json.load(f)
    
    updated_count = 0
    total_chars = 0
    for pinyin, items in data.items():
        for item in items:
            total_chars += 1
            char = item.get('char')
            if char in char_map:
                item['stroke_aux'] = char_map[char]
                updated_count += 1
    
    with open(file_path, 'w', encoding='utf-8') as f:
        json.dump(data, f, ensure_ascii=False, indent=2)
    
    print(f"Updated {updated_count}/{total_chars} characters in {file_path}")

if __name__ == "__main__":
    char_map = load_map('stroke_map.txt')
    update_dict('dicts/chinese/chars/chars.json', char_map)
    update_dict('dicts/chinese/chars/level2.json', char_map)
    update_dict('dicts/chinese/chars/level3.json', char_map)
