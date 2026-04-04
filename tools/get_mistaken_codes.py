import json
import os

def get_codes():
    path = 'dicts/stroke/words/stroke_char.json'
    chars = [
        "火", "方", "为", "片", "九", "万", "比", "再", "凸", "凹", "母",
        "具", "冒", "肺", "黄", "考", "周", "喜", "身", "燕", "录", "没",
        "寻", "帚", "雪", "妇", "建", "唐", "康", "临", "巷", "窗",
        "免", "奂", "象", "鬼", "卑", "畏", "展", "代", "武", "贰",
        "市", "沛", "尧", "步", "染", "琴", "纸", "义", "叉", "发", "成"
    ]
    
    if not os.path.exists(path):
        print("Error: stroke_char.json not found")
        return

    with open(path, 'r', encoding='utf-8') as f:
        data = json.load(f)
    
    char_to_code = {}
    for code, entries in data.items():
        for entry in entries:
            c = entry['char']
            if c in chars:
                # 笔画编码可能由于简码存在重复，我们只取最短的那个（代表单字本身编码）
                if c not in char_to_code or len(code) < len(char_to_code[c]):
                    char_to_code[c] = code

    for c in chars:
        print(f"{c}: {char_to_code.get(c, 'N/A')}")

if __name__ == "__main__":
    get_codes()
