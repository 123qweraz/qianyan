import json
import os

def update_chars_with_weights():
    rime_yaml = "dicts/chinese/base.dict.yaml"
    chars_json_path = "dicts/chinese/chars/chars.json"
    
    # 1. 加载 Rime 里的单字数据
    # (char, pinyin) -> weight
    char_weight_map = {}
    print(f"Loading character weights from {rime_yaml}...")
    with open(rime_yaml, 'r', encoding='utf-8') as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith('#'): continue
            parts = line.split('	')
            if len(parts) >= 2:
                word = parts[0]
                # 只处理单字
                if len(word) == 1:
                    pinyin = parts[1].replace(' ', '').to_lowercase()
                    weight = int(parts[2]) if len(parts) > 2 else 0
                    char_weight_map[(word, pinyin)] = weight

    # 2. 读取并更新 chars.json
    print(f"Updating {chars_json_path}...")
    with open(chars_json_path, 'r', encoding='utf-8') as f:
        old_chars_data = json.load(f)

    new_chars_data = {}
    
    # 我们遍历原有的单字库，确保现有的注释/分类不丢失
    for old_py, entries in old_chars_data.items():
        for entry in entries:
            char = entry['char']
            # 尝试在 Rime 中寻找匹配
            # 因为一个字在原有库里的拼音可能和 Rime 不同，
            # 我们先按 (char, old_py) 找，找不到就只按 char 找
            weight = char_weight_map.get((char, old_py))
            
            # 如果 Rime 明确有这个读音，更新它
            if weight is not None:
                entry['weight'] = weight
                target_py = old_py
            else:
                # 如果没找到，尝试在 Rime 中找这个字的所有读音
                rime_pinyins = [k[1] for k in char_weight_map.keys() if k[0] == char]
                if rime_pinyins:
                    # 如果 Rime 有读音，我们取第一个匹配的，或者保留原样
                    entry['weight'] = char_weight_map.get((char, rime_pinyins[0]), 10)
                    target_py = old_py # 暂时保持原读音分类，除非你明确想大改
                else:
                    entry['weight'] = 10
                    target_py = old_py

            if target_py not in new_chars_data:
                new_chars_data[target_py] = []
            new_chars_data[target_py].append(entry)

    # 排序：按权重降序
    for py in new_chars_data:
        new_chars_data[py].sort(key=lambda x: x.get('weight', 0), reverse=True)

    # 3. 写回文件
    print("Saving updated chars.json...")
    with open(chars_json_path, 'w', encoding='utf-8') as f:
        json.dump(new_chars_data, f, ensure_ascii=False, indent=2)

    print("Success! Single characters now have weights.")

if __name__ == "__main__":
    update_chars_with_weights()
