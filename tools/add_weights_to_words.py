import json
import os

def add_weights_to_json(existing_json_path, rime_yaml_path, output_json_path):
    # 1. 建立权重映射：(词组, 拼音) -> 权重
    # 使用 (word, pinyin) 作为 key 是因为同音词权重不同
    weight_map = {}
    print(f"Loading weights from {rime_yaml_path}...")
    with open(rime_yaml_path, 'r', encoding='utf-8') as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith('#'):
                continue
            parts = line.split('	')
            if len(parts) >= 3:
                word = parts[0]
                pinyin = parts[1].replace(' ', '') # 移除空格匹配 json 的 key
                weight = int(parts[2])
                weight_map[(word, pinyin)] = weight

    # 2. 读取并更新现有的 words.json
    print(f"Updating {existing_json_path}...")
    if not os.path.exists(existing_json_path):
        print("Error: words.json not found!")
        return

    with open(existing_json_path, 'r', encoding='utf-8') as f:
        data = json.load(f)

    updated_count = 0
    not_found_count = 0

    for pinyin, entries in data.items():
        for entry in entries:
            word = entry.get('char')
            if word:
                # 尝试匹配权重
                weight = weight_map.get((word, pinyin))
                if weight is not None:
                    entry['weight'] = weight
                    updated_count += 1
                else:
                    # 如果没找到，给一个较小的默认权重
                    entry['weight'] = 10
                    not_found_count += 1

    print(f"Update complete. Matched: {updated_count}, Defaulted: {not_found_count}")

    # 3. 保存更新后的文件
    with open(output_json_path, 'w', encoding='utf-8') as f:
        json.dump(data, f, ensure_ascii=False, indent=2)
    
    print(f"Saved updated dictionary to {output_json_path}")

if __name__ == "__main__":
    existing_json = "dicts/chinese/words/words.json"
    rime_yaml = "dicts/chinese/base.dict.yaml"
    # 直接覆盖原文件或存为新文件，这里先存为新文件供你检查
    output_json = "dicts/chinese/words/words_with_weights.json"
    
    add_weights_to_json(existing_json, rime_yaml, output_json)
