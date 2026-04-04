import json
import os

def extract_unique_words(existing_json_path, rime_yaml_path, output_json_path, min_weight=1000):
    # 1. 加载现有词典中的所有词组
    existing_words = set()
    if os.path.exists(existing_json_path):
        print(f"Loading existing words from {existing_json_path}...")
        with open(existing_json_path, 'r', encoding='utf-8') as f:
            data = json.load(f)
            for pinyin in data:
                for item in data[pinyin]:
                    if isinstance(item, dict) and 'char' in item:
                        existing_words.add(item['char'])
                    elif isinstance(item, str):
                        existing_words.add(item)
    
    print(f"Total existing words: {len(existing_words)}")

    # 2. 解析 YAML 并过滤
    new_data = {}
    new_count = 0
    
    print(f"Processing {rime_yaml_path}...")
    with open(rime_yaml_path, 'r', encoding='utf-8') as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith('#'):
                continue
            
            parts = line.split('	')
            if len(parts) < 2:
                continue
                
            word = parts[0]
            # 跳过已存在的词
            if word in existing_words:
                continue
                
            pinyin = parts[1].replace(' ', '')
            weight = int(parts[2]) if len(parts) > 2 else 0
            
            if weight >= min_weight:
                if pinyin not in new_data:
                    new_data[pinyin] = []
                
                # 构造与 words.json 兼容的结构
                new_data[pinyin].append({
                    "char": word,
                    "weight": weight,
                    "category": "rime_new"
                })
                new_count += 1

    # 3. 排序并保存
    # 同一拼音下按权重排序
    for py in new_data:
        new_data[py].sort(key=lambda x: x['weight'], reverse=True)

    print(f"Found {new_count} new unique words.")
    
    with open(output_json_path, 'w', encoding='utf-8') as f:
        json.dump(new_data, f, ensure_ascii=False, indent=2)
    
    print(f"Saved to {output_json_path}")

if __name__ == "__main__":
    existing_json = "dicts/chinese/words/words.json"
    rime_yaml = "dicts/chinese/base.dict.yaml"
    output_json = "dicts/chinese/new_words.json"
    
    extract_unique_words(existing_json, rime_yaml, output_json, min_weight=1000)
