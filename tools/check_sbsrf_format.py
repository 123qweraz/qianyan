import os

def parse_bihua():
    file_path = 'reference/sbsrf/sbxlm/bihua.dict.yaml'
    
    # 根据结果推导：
    # 涓€ (一) -> a (3)  - 哎呀，还是乱码，我用 hex 重新测
    # 我们打印 hex 对应笔画代码
    
    char_map = {}
    with open(file_path, 'r', encoding='utf-8') as f:
        in_data = False
        for line in f:
            line = line.strip()
            if not in_data:
                if line == '...': in_data = True
                continue
            if not line or line.startswith('#'): continue
            
            parts = line.split()
            if len(parts) >= 2:
                char = parts[0]
                code = parts[1]
                char_map[char] = code
    return char_map

if __name__ == "__main__":
    res = parse_bihua()
    # 典型字验证映射关系
    # 一 (0x4e00): 横
    # 丨 (0x4e28): 竖
    # 丿 (0x4e3f): 撇
    # 丶 (0x4e36): 捺
    # 乙 (0x4e59): 折
    
    targets = {
        "\u4e00": "一",
        "\u4e28": "丨",
        "\u4e3f": "丿",
        "\u4e36": "丶",
        "\u4e59": "乙",
        "\u5341": "十",
        "\u4eba": "人",
        "\u53e3": "口",
    }
    
    for hex_char, name in targets.items():
        if hex_char in res:
            print(f"{name} ({hex_char}): {res[hex_char]}")
