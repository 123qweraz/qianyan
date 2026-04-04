import json
import os

# 1. 笔画映射 (sbsrf -> 标准数字 1-5)
# e:横(1), i:竖(2), u:撇(3), o:捺(4), a:折(5)
SB_MAP = {'e': 1, 'i': 2, 'u': 3, 'o': 4, 'a': 5}

# 2. 你的 5x5 键盘矩阵 (首笔区 x 次笔区)
# 横(1): G F D S A
# 竖(2): H J K L M
# 撇(3): T R E W Q
# 捺(4): Y U I O P
# 折(5): N B V C X
MATRIX = {
    1: {1: 'g', 2: 'f', 3: 'd', 4: 's', 5: 'a'},
    2: {1: 'h', 2: 'j', 3: 'k', 4: 'l', 5: 'm'},
    3: {1: 't', 2: 'r', 3: 'e', 4: 'w', 5: 'q'},
    4: {1: 'y', 2: 'u', 3: 'i', 4: 'o', 5: 'p'},
    5: {1: 'n', 2: 'b', 3: 'v', 4: 'c', 5: 'x'}
}

def load_sbsrf_data():
    file_path = 'reference/sbsrf/sbxlm/bihua.dict.yaml'
    char_map = {}
    with open(file_path, 'r', encoding='utf-8') as f:
        in_data = False
        for line in f:
            line = line.strip()
            if line == '...': in_data = True; continue
            if not in_data or not line or line.startswith('#'): continue
            parts = line.split()
            if len(parts) >= 2:
                char = parts[0]
                code = parts[1]
                
                # 获取前两笔
                s1 = SB_MAP.get(code[0])
                s2 = SB_MAP.get(code[1]) if len(code) >= 2 else None
                
                # 获取末两笔
                if len(code) >= 4:
                    s3 = SB_MAP.get(code[-2])
                    s4 = SB_MAP.get(code[-1])
                elif len(code) == 3:
                    s3 = SB_MAP.get(code[-1])
                    s4 = None
                else:
                    s3 = None
                    s4 = None
                
                aux = ""
                if s1:
                    if s2:
                        aux += MATRIX[s1][s2]
                    else:
                        # 只有一笔，补横（或者也可以只用s1所在的某个默认键，这里补s1+横）
                        aux += MATRIX[s1][1]
                
                if s3:
                    if s4:
                        aux += MATRIX[s3][s4]
                    else:
                        aux += MATRIX[s3][1]
                
                if aux:
                    char_map[char] = aux.capitalize()
    return char_map

def process_file(file_path, stroke_map):
    print(f"Updating {file_path} with stroke aux codes...")
    if not os.path.exists(file_path): return
    with open(file_path, 'r', encoding='utf-8') as f:
        data = json.load(f)
    
    count = 0
    for key in data:
        for entry in data[key]:
            char = entry.get('char')
            if char in stroke_map:
                entry['stroke_aux'] = stroke_map[char]
                count += 1
    
    with open(file_path, 'w', encoding='utf-8') as f:
        json.dump(data, f, ensure_ascii=False, indent=2)
    print(f"Updated {count} characters in {file_path}.")

if __name__ == "__main__":
    stroke_map = load_sbsrf_data()
    files = [
        "dicts/chinese/chars/chars.json",
        "dicts/chinese/chars/level2.json",
        "dicts/chinese/chars/level3.json",
        "dicts/chinese/chars/rime_mint_chars.json"
    ]
    for f in files:
        process_file(f, stroke_map)
