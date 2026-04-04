import json
import csv
import os

def enrich_stroke_dict():
    # 路径定义
    csv_path = 'referdict/Chinese character list from 2.5 billion words corpus ordered by frequency.csv'
    stroke_json_path = 'dicts/stroke/words/stroke_char.json'
    chars_json_paths = [
        'dicts/chinese/chars/chars.json',
        'dicts/chinese/chars/level2.json',
        'dicts/chinese/chars/level3.json'
    ]
    
    # 1. 加载字频 (来自语料库 CSV)
    char_weights = {}
    if os.path.exists(csv_path):
        print(f"正在读取语料库: {csv_path}")
        with open(csv_path, 'r', encoding='utf-8') as f:
            lines = f.readlines()
            reader = csv.reader(lines[1:])
            for row in reader:
                if len(row) >= 3:
                    char = row[1]
                    try:
                        char_weights[char] = int(row[2])
                    except: continue
    
    # 2. 加载拼音和翻译信息 (来自中文单字库)
    char_info = {}
    for path in chars_json_paths:
        if os.path.exists(path):
            print(f"正在读取参考词典: {path}")
            with open(path, 'r', encoding='utf-8') as f:
                try:
                    data = json.load(f)
                    # 遍历拼音 Key 结构
                    for py, entries in data.items():
                        for entry in entries:
                            c = entry.get('char')
                            if not c: continue
                            
                            # 提取拼音 (优先取 tone，如果没有则用 key)
                            tone = entry.get('tone', py)
                            # 如果 tone 是 a/ā/á 这种形式，取第一个有声调的
                            if '/' in tone:
                                parts = tone.split('/')
                                tone = parts[1] if len(parts) > 1 else parts[0]
                            
                            en = entry.get('en', '')
                            trad = entry.get('trad', c)
                            
                            # 存储 (如果已存在，保留权重较高的或第一个)
                            if c not in char_info:
                                char_info[c] = {
                                    "tone": tone,
                                    "en": en,
                                    "trad": trad
                                }
                except Exception as e:
                    print(f"解析 {path} 失败: {e}")

    # 3. 读取并丰富笔画词典
    if os.path.exists(stroke_json_path):
        print(f"正在处理笔画词典: {stroke_json_path}")
        with open(stroke_json_path, 'r', encoding='utf-8') as f:
            stroke_dict = json.load(f)
    else:
        print("错误: 未找到笔画词典 JSON 文件")
        return

    new_stroke_dict = {}
    for code, chars in stroke_dict.items():
        enriched_chars = []
        for item in chars:
            # 处理旧格式或新格式
            char = item['char'] if isinstance(item, dict) else item
            
            info = char_info.get(char, {})
            weight = char_weights.get(char, item.get('weight', 1) if isinstance(item, dict) else 1)
            
            entry = {
                "char": char,
                "weight": weight,
                "tone": info.get('tone', ''),
                "trad": info.get('trad', char),
                "en": info.get('en', '')
            }
            enriched_chars.append(entry)
            
        # 排序：按权重降序
        enriched_chars.sort(key=lambda x: x['weight'], reverse=True)
        new_stroke_dict[code] = enriched_chars

    # 4. 写回
    print(f"正在保存丰富后的词典...")
    with open(stroke_json_path, 'w', encoding='utf-8') as f:
        json.dump(new_stroke_dict, f, ensure_ascii=False, indent=2)
    
    print("笔画词典拼音与信息丰富完成！")

if __name__ == "__main__":
    enrich_stroke_dict()
