import json
import os

def load_char_stroke_map():
    # 从 chars.json, level2.json, level3.json 中加载所有单字到笔画首位字母的映射
    char_map = {}
    char_files = [
        "dicts/chinese/chars/chars.json",
        "dicts/chinese/chars/level2.json",
        "dicts/chinese/chars/level3.json"
    ]
    for file in char_files:
        if not os.path.exists(file): continue
        with open(file, 'r', encoding='utf-8') as f:
            data = json.load(f)
            for pinyin_key in data:
                for entry in data[pinyin_key]:
                    char = entry.get('char')
                    stroke_aux = entry.get('stroke_aux', '')
                    if char and stroke_aux:
                        # 我们取第一个字母 (代表前两笔)
                        # 为了统一，我们将其转换为大写 (代表二字词的第一和第二部分)
                        char_map[char] = stroke_aux[0].upper()
    return char_map

def process_words(word_file, char_map):
    print(f"Processing {word_file}...")
    if not os.path.exists(word_file): return
    
    with open(word_file, 'r', encoding='utf-8') as f:
        data = json.load(f)
    
    count = 0
    for pinyin_key in data:
        for entry in data[pinyin_key]:
            word = entry.get('char', '')
            # 只处理二字词
            if len(word) == 2:
                c1 = word[0]
                c2 = word[1]
                if c1 in char_map and c2 in char_map:
                    # 规则：首字头两笔字母 + 次字头两笔字母
                    # 注意：如果原本就有 stroke_aux，我们只更新或在必要时覆盖
                    entry['stroke_aux'] = char_map[c1] + char_map[c2]
                    count += 1
    
    with open(word_file, 'w', encoding='utf-8') as f:
        json.dump(data, f, ensure_ascii=False, indent=2)
    
    print(f"Finished. Updated {count} two-character words.")

if __name__ == "__main__":
    char_map = load_char_stroke_map()
    process_words("dicts/chinese/words/words.json", char_map)
    # 也处理其他可能存在的词典
    process_words("dicts/chinese/words/new_words.json", char_map)
    process_words("dicts/chinese/words/words_jianpin.json", char_map)
