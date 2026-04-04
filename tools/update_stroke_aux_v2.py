import json
import os
import re

# 1. 笔画矩阵 (SBSRF 4码逻辑)
# 1:横(G-A), 2:竖(H-M), 3:撇(T-Q), 4:点/捺(Y-P), 5:折(N-X)
MATRIX = {
    1: {1: 'g', 2: 'f', 3: 'd', 4: 's', 5: 'a'},
    2: {1: 'h', 2: 'j', 3: 'k', 4: 'l', 5: 'm'},
    3: {1: 't', 2: 'r', 3: 'e', 4: 'w', 5: 'q'},
    4: {1: 'y', 2: 'u', 3: 'i', 4: 'o', 5: 'p'},
    5: {1: 'n', 2: 'b', 3: 'v', 4: 'c', 5: 'x'}
}

def get_stroke_aux(strokes):
    if not strokes:
        return ""
    
    # 将字符串笔画转换为数字列表
    s = [int(c) for c in strokes if c in "12345"]
    if not s:
        return ""

    n = len(s)
    s1 = s[0]
    s2 = s[1] if n >= 2 else 1
    
    # 第一部分码
    part1 = MATRIX[s1][s2]
    
    # 第二部分码 (如果有 3 笔及以上)
    if n >= 4:
        s3 = s[-2]
        s4 = s[-1]
        part2 = MATRIX[s3][s4]
        return (part1 + part2).capitalize()
    elif n == 3:
        s3 = s[-1]
        part2 = MATRIX[s3][1]
        return (part1 + part2).capitalize()
    else:
        # 1 或 2 笔，只有一部分码
        return part1.upper()

def load_data_v2():
    print("Loading strokes from reference/bh/data_v2.js...")
    char_map = {}
    path = "reference/bh/data_v2.js"
    if not os.path.exists(path):
        print(f"Error: {path} not found.")
        return {}
    
    with open(path, "r", encoding="utf-8") as f:
        content = f.read()
        # 匹配 ["汉字","笔画序列"]
        matches = re.findall(r'\["(.*?)","(\d+)"\]', content)
        for char, strokes in matches:
            char_map[char] = get_stroke_aux(strokes)
    
    print(f"Loaded {len(char_map)} characters.")
    return char_map

def update_dictionary(file_path, char_map):
    print(f"Updating {file_path}...")
    if not os.path.exists(file_path):
        return
    
    with open(file_path, "r", encoding="utf-8") as f:
        data = json.load(f)
    
    updated_count = 0
    missing_count = 0
    
    for pinyin in data:
        for entry in data[pinyin]:
            char = entry.get("char")
            if char in char_map:
                entry["stroke_aux"] = char_map[char]
                updated_count += 1
            else:
                missing_count += 1
    
    with open(file_path, "w", encoding="utf-8") as f:
        json.dump(data, f, ensure_ascii=False, indent=2)
    
    print(f"Finished. Updated: {updated_count}, Missing: {missing_count}")

if __name__ == "__main__":
    char_map = load_data_v2()
    if char_map:
        files = [
            "dicts/chinese/chars/chars.json",
            "dicts/chinese/chars/level2.json",
            "dicts/chinese/chars/level3.json"
        ]
        for f in files:
            update_dictionary(f, char_map)
