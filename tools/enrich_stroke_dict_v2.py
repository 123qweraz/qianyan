import json
import csv
import os

def enrich_stroke_dict_v2():
    # 路径定义
    csv_path = 'referdict/Chinese character list from 2.5 billion words corpus ordered by frequency.csv'
    stroke_json_path = 'dicts/stroke/words/stroke_char.json'
    chars_json_paths = [
        ('dicts/chinese/chars/chars.json', 'level-1'),
        ('dicts/chinese/chars/level2.json', 'level-2'),
        ('dicts/chinese/chars/level3.json', 'level-3')
    ]
    
    # 1. 加载字频
    char_weights = {}
    if os.path.exists(csv_path):
        print(f"正在读取语料库: {csv_path}")
        with open(csv_path, 'r', encoding='utf-8') as f:
            lines = f.readlines()
            reader = csv.reader(lines[1:])
            for row in reader:
                if len(row) >= 3:
                    char = row[1]
                    try: char_weights[char] = int(row[2])
                    except: continue
    
    # 2. 加载级别、拼音和翻译信息
    char_info = {}
    for path, default_lvl in chars_json_paths:
        if os.path.exists(path):
            print(f"正在读取参考词典: {path}")
            with open(path, 'r', encoding='utf-8') as f:
                try:
                    data = json.load(f)
                    for py, entries in data.items():
                        for entry in entries:
                            c = entry.get('char')
                            if not c: continue
                            
                            # 提取声调
                            tone = entry.get('tone', py)
                            if '/' in tone:
                                parts = tone.split('/')
                                tone = parts[1] if len(parts) > 1 else parts[0]
                            
                            # 确定级别
                            lvl = entry.get('category', default_lvl)
                            
                            if c not in char_info:
                                char_info[c] = {
                                    "tone": tone,
                                    "en": entry.get('en', ''),
                                    "trad": entry.get('trad', c),
                                    "category": lvl
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

    # 3.1 处理全码和注入级别
    processed_dict = {}
    for code, chars in stroke_dict.items():
        enriched_chars = []
        for item in chars:
            char = item['char'] if isinstance(item, dict) else item
            info = char_info.get(char, {})
            weight = char_weights.get(char, item.get('weight', 1) if isinstance(item, dict) else 1)
            
            entry = {
                "char": char, "weight": weight,
                "tone": info.get('tone', ''), "trad": info.get('trad', char),
                "en": info.get('en', ''), "category": info.get('category', 'rare')
            }
            enriched_chars.append(entry)
        enriched_chars.sort(key=lambda x: x['weight'], reverse=True)
        processed_dict[code] = enriched_chars

    # 3.2 生成 4 键简码 (前三末一)
    print("正在生成单字 4 键简码...")
    short_stroke_dict = {}
    for code, entries in processed_dict.items():
        if len(code) > 4:
            short_code = code[:3] + code[-1]
            if short_code not in short_stroke_dict:
                short_stroke_dict[short_code] = []
            
            # 避免重复添加
            existing_chars = {e['char'] for e in short_stroke_dict[short_code]}
            for entry in entries:
                if entry['char'] not in existing_chars:
                    short_stroke_dict[short_code].append(entry)

    # 4. 写回
    print(f"正在保存全码词典: {stroke_json_path}")
    with open(stroke_json_path, 'w', encoding='utf-8') as f:
        json.dump(processed_dict, f, ensure_ascii=False, indent=2)

    short_json_path = 'dicts/stroke/words/stroke_short.json'
    print(f"正在保存简码词典: {short_json_path}")
    for code in short_stroke_dict:
        short_stroke_dict[code].sort(key=lambda x: x['weight'], reverse=True)
    with open(short_json_path, 'w', encoding='utf-8') as f:
        json.dump(short_stroke_dict, f, ensure_ascii=False, indent=2)
    
    print("笔画词典 V2 拆分保存完成！")

if __name__ == "__main__":
    enrich_stroke_dict_v2()
