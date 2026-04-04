import json
import os

def list_unmatched_words(existing_json_path, rime_yaml_path, output_txt_path):
    # 1. 建立权重映射：(词组, 拼音)
    weight_map = set()
    print(f"Loading words from {rime_yaml_path}...")
    with open(rime_yaml_path, 'r', encoding='utf-8') as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith('#'):
                continue
            parts = line.split('\t')
            if len(parts) >= 2:
                word = parts[0]
                pinyin = parts[1].replace(' ', '')
                weight_map.add((word, pinyin))

    # 2. 检查 words.json
    print(f"Checking {existing_json_path}...")
    if not os.path.exists(existing_json_path):
        print("Error: words.json not found!")
        return

    with open(existing_json_path, 'r', encoding='utf-8') as f:
        data = json.load(f)

    unmatched = []

    for pinyin, entries in data.items():
        for entry in entries:
            word = entry.get('char')
            if word:
                if (word, pinyin) not in weight_map:
                    unmatched.append(f"{word}\t{pinyin}")

    print(f"Found {len(unmatched)} unmatched words.")

    # 3. 保存到 txt
    with open(output_txt_path, 'w', encoding='utf-8') as f:
        for line in unmatched:
            f.write(line + "\n")
    
    print(f"Saved unmatched words to {output_txt_path}")

if __name__ == "__main__":
    existing_json = "dicts/chinese/words/words.json"
    rime_yaml = "dicts/chinese/base.dict.yaml"
    output_txt = "unmatched_words.txt"
    
    list_unmatched_words(existing_json, rime_yaml, output_txt)