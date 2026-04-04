import json
import os
import re

def is_chinese(text):
    return re.search(r'[\u4e00-\u9fff]', text)

def audit_file(file_path):
    if not os.path.exists(file_path):
        return
    
    print(f"\n===== 正在审计: {file_path} =====")
    with open(file_path, 'r', encoding='utf-8') as f:
        data = json.load(f)

    suspicious_count = 0
    
    for pinyin, entries in data.items():
        seen_hints = {}
        for entry in entries:
            word = entry.get('char', '')
            hint = entry.get('en', '')
            
            reasons = []
            
            # 1. 检查是否为空
            if not hint:
                reasons.append("缺失英文提示")
            
            # 2. 检查英文中是否包含中文
            if is_chinese(hint):
                reasons.append("提示中包含中文字符")
            
            # 3. 检查提示是否只是拼音 (忽略大小写)
            if hint.lower().replace(' ', '') == pinyin.lower():
                reasons.append("提示与拼音相同 (可能未翻译)")
            
            # 4. 检查同一拼音下的重复提示
            if hint in seen_hints:
                reasons.append(f"提示词与 '{seen_hints[hint]}' 重复")
            else:
                seen_hints[hint] = word

            if reasons:
                suspicious_count += 1
                if suspicious_count <= 50: # 仅打印前50个例子
                    print(f"条目 [{word} | {pinyin}]: {', '.join(reasons)} (Hint: {hint})")
                elif suspicious_count == 51:
                    print("... 更多可疑条目已省略 ...")

    print(f"审计完成。共发现 {suspicious_count} 个可疑条目。")

if __name__ == "__main__":
    targets = [
        'dicts/chinese/words/words.json',
        'dicts/chinese/words/capital_word/name.json',
        'dicts/chinese/words/capital_word/place.json'
    ]
    for t in targets:
        audit_file(t)
