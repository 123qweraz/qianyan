import json
import csv
import os

def update_stroke_weights():
    csv_path = 'referdict/Chinese character list from 2.5 billion words corpus ordered by frequency.csv'
    stroke_json_path = 'dicts/stroke/words/stroke_char.json'
    
    # 1. 加载字频
    char_weights = {}
    if os.path.exists(csv_path):
        print(f"正在读取语料库: {csv_path}")
        with open(csv_path, 'r', encoding='utf-8') as f:
            # 跳过第一行 (BOM 或 标题)
            lines = f.readlines()
            reader = csv.reader(lines[1:])
            for row in reader:
                if len(row) >= 3:
                    char = row[1]
                    try:
                        # 使用 token 数作为权重
                        weight = int(row[2])
                        char_weights[char] = weight
                    except:
                        continue
    else:
        print("错误: 未找到语料库 CSV 文件")
        return

    # 2. 读取笔画词典
    if os.path.exists(stroke_json_path):
        print(f"正在读取笔画词典: {stroke_json_path}")
        with open(stroke_json_path, 'r', encoding='utf-8') as f:
            stroke_dict = json.load(f)
    else:
        print("错误: 未找到笔画词典 JSON 文件")
        return

    # 3. 注入权重
    new_stroke_dict = {}
    for code, chars in stroke_dict.items():
        new_chars = []
        for char in chars:
            weight = char_weights.get(char, 0)
            # 如果是部首/特殊字符，权重设低点，但保留
            if weight == 0:
                weight = 1
            new_chars.append({
                "char": char,
                "weight": weight
            })
        # 排序：按权重降序
        new_chars.sort(key=lambda x: x['weight'], reverse=True)
        new_stroke_dict[code] = new_chars

    # 4. 写回
    print(f"正在保存更新后的词典...")
    with open(stroke_json_path, 'w', encoding='utf-8') as f:
        json.dump(new_stroke_dict, f, ensure_ascii=False, indent=2)
    
    print("笔画词典字频注入完成！")

if __name__ == "__main__":
    update_stroke_weights()
