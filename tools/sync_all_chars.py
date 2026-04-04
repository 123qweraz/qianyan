import json
import os

def update_all_char_files():
    rime_yaml = "dicts/chinese/chars/8105.dict.yaml"
    char_files = [
        "dicts/chinese/chars/chars.json",
        "dicts/chinese/chars/level2.json",
        "dicts/chinese/chars/level3.json"
    ]
    
    # 1. 建立 Rime 映射: char -> list of (pinyin, weight)
    rime_map = {}
    print(f"Loading data from {rime_yaml}...")
    with open(rime_yaml, 'r', encoding='utf-8') as f:
        in_data = False
        for line in f:
            line = line.strip()
            if line == "...":
                in_data = True
                continue
            if not in_data:
                if '	' in line: in_data = True # 自动探测
                else: continue
            
            if line.startswith('#'): continue
            parts = line.split('	')
            if len(parts) >= 2:
                char = parts[0]
                pinyin = parts[1].replace(' ', '').to_lowercase()
                weight = int(parts[2]) if len(parts) > 2 else 0
                
                if char not in rime_map:
                    rime_map[char] = []
                rime_map[char].append((pinyin, weight))

    # 2. 处理每一个 JSON 文件
    for json_path in char_files:
        if not os.path.exists(json_path):
            print(f"Skipping missing file: {json_path}")
            continue
            
        print(f"Processing {json_path}...")
        with open(json_path, 'r', encoding='utf-8') as f:
            data = json.load(f)
            
        # 收集该文件中所有的字，以及它们对应的元数据（en, category, tone 等）
        # char -> list of entries
        char_metadata = {}
        for py, entries in data.items():
            for entry in entries:
                c = entry['char']
                if c not in char_metadata:
                    char_metadata[c] = []
                # 保存除了 char 之外的所有属性
                meta = entry.copy()
                char_metadata[c].append((py, meta))

        new_data = {}

        # 我们以 Rime 的数据为骨架来重构，但保留原有的元数据
        for char, rime_entries in rime_map.items():
            # 只有当这个字在当前 JSON 文件中存在时才处理（保持 level2/level3 的物理隔离）
            if char in char_metadata:
                for r_py, r_weight in rime_entries:
                    # 查找是否有匹配原读音的元数据
                    matched_meta = None
                    for old_py, meta in char_metadata[char]:
                        if old_py == r_py:
                            matched_meta = meta
                            break
                    
                    if not matched_meta:
                        # 如果拼音变了，拿第一个可用的元数据作为参考
                        matched_meta = char_metadata[char][0][1].copy()
                        # 更新提示音（如果存在）
                        if 'tone' in matched_meta:
                            matched_meta['tone'] = r_py 

                    final_entry = matched_meta.copy()
                    final_entry['weight'] = r_weight
                    
                    if r_py not in new_data:
                        new_data[r_py] = []
                    new_data[r_py].append(final_entry)

        # 排序
        for py in new_data:
            new_data[py].sort(key=lambda x: x.get('weight', 0), reverse=True)

        # 写回
        with open(json_path, 'w', encoding='utf-8') as f:
            json.dump(new_data, f, ensure_ascii=False, indent=2)
            
    print("All character JSON files updated and synced with Rime weights/pinyins.")

if __name__ == "__main__":
    update_all_char_files()
