import json
import os
from pypinyin import pinyin, Style

# 笔画映射: 1:横, 2:竖, 3:撇, 4:点/捺, 5:折
# 我们需要把 (笔画1, 笔画2) 映射到 25 个字母 (A-Y)
# 映射公式: (stroke1 - 1) * 5 + (stroke2 - 1) -> 0-24
# 0 -> A, 1 -> B, ..., 24 -> Y

def get_stroke_code(char):
    try:
        # pinyin(char, style=Style.STROKE) 返回 [['12345']] 形式的字符串
        s_list = pinyin(char, style=Style.STROKE)
        if not s_list or not s_list[0] or not s_list[0][0]:
            return ""
        
        strokes = s_list[0][0]
        if len(strokes) < 1:
            return ""
            
        # 获取前两笔，如果只有一笔，第二笔视为 0 (或者重复第一笔？通常笔画辅助码会补位)
        # 这里我们假设如果只有一笔，第二笔默认为 1 (横) 或者直接用单笔
        s1 = int(strokes[0])
        s2 = int(strokes[1]) if len(strokes) >= 2 else 1 # 补位默认横
        
        # 确保在 1-5 范围内 (pypinyin 的 stroke 模式通常是 1-5)
        s1 = max(1, min(5, s1))
        s2 = max(1, min(5, s2))
        
        idx = (s1 - 1) * 5 + (s2 - 1)
        return chr(ord('a') + idx)
    except Exception:
        return ""

def process_file(file_path):
    print(f"Processing {file_path} for stroke aux codes...")
    if not os.path.exists(file_path):
        print(f"File {file_path} not found.")
        return

    with open(file_path, 'r', encoding='utf-8') as f:
        data = json.load(f)

    count = 0
    for key in data:
        for entry in data[key]:
            char = entry.get('char')
            if char:
                aux = get_stroke_code(char)
                if aux:
                    # 我们把笔画码存入 'aux' 字段，或者更新 'en' 字段作为提示
                    # 按照用户要求，这是一种辅助码，我们存入 'stroke_aux'
                    entry['stroke_aux'] = aux
                    count += 1
    
    with open(file_path, 'w', encoding='utf-8') as f:
        json.dump(data, f, ensure_ascii=False, indent=2)
    
    print(f"Finished {file_path}. Updated {count} characters.")

if __name__ == "__main__":
    files = [
        "dicts/chinese/chars/chars.json",
        "dicts/chinese/chars/level2.json",
        "dicts/chinese/chars/level3.json"
    ]
    for f in files:
        process_file(f)
