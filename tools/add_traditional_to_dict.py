import json
import os
from opencc import OpenCC

# 初始化 OpenCC，使用 s2t (Simplified to Traditional) 转换配置
cc = OpenCC('s2t')

def process_file(file_path):
    if not os.path.exists(file_path): return
    print(f"Processing {file_path} with OpenCC...")
    with open(file_path, 'r', encoding='utf-8') as f:
        data = json.load(f)
    
    count = 0
    for pinyin in data:
        for entry in data[pinyin]:
            s_word = entry.get('char', '')
            # 使用 OpenCC 进行精准转换
            entry['trad'] = cc.convert(s_word)
            count += 1
            
    with open(file_path, 'w', encoding='utf-8') as f:
        json.dump(data, f, ensure_ascii=False, indent=2)
    print(f"Done. Updated {count} entries.")

if __name__ == "__main__":
    files = [
        "dicts/chinese/chars/chars.json",
        "dicts/chinese/chars/level2.json",
        "dicts/chinese/chars/level3.json",
        "dicts/chinese/words/words.json"
    ]
    for f in files:
        process_file(f)
