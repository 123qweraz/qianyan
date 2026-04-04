import json
import os

def update_json_file_directly(json_path, rime_weights):
    if not os.path.exists(json_path):
        return
    
    print(f"Updating {json_path}...")
    with open(json_path, 'r', encoding='utf-8') as f:
        data = json.load(f)
    
    updated_count = 0
    for py, entries in data.items():
        for entry in entries:
            char = entry['char']
            # 尝试匹配权重 (char, pinyin)
            weight = rime_weights.get((char, py))
            if weight is None:
                # 尝试只匹配 char
                possible_weights = [w for (c, p), w in rime_weights.items() if c == char]
                weight = max(possible_weights) if possible_weights else 10
            
            entry['weight'] = weight
            updated_count += 1
            
        entries.sort(key=lambda x: x.get('weight', 0), reverse=True)

    with open(json_path, 'w', encoding='utf-8') as f:
        json.dump(data, f, ensure_ascii=False, indent=2)
    print(f"Finished {json_path}, updated {updated_count} entries.")

def main():
    rime_yaml = "dicts/chinese/chars/8105.dict.yaml"
    char_files = [
        "dicts/chinese/chars/chars.json",
        "dicts/chinese/chars/level2.json",
        "dicts/chinese/chars/level3.json"
    ]
    
    rime_weights = {}
    print(f"Reading weights from {rime_yaml}...")
    with open(rime_yaml, 'r', encoding='utf-8') as f:
        data_started = False
        for line in f:
            line = line.strip()
            if line == "...":
                data_started = True
                continue
            if not data_started:
                if '\t' in line and not line.startswith('#'):
                    data_started = True
                else:
                    continue
            
            parts = line.split('\t')
            if len(parts) >= 2:
                char = parts[0]
                pinyin = parts[1].replace(' ', '').lower()
                try:
                    weight = int(parts[2]) if len(parts) > 2 else 0
                except ValueError:
                    weight = 0
                rime_weights[(char, pinyin)] = weight
    
    print(f"Loaded {len(rime_weights)} weights from Rime.")
    
    for f_path in char_files:
        update_json_file_directly(f_path, rime_weights)

if __name__ == "__main__":
    main()