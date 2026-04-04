import json
import os

def main():
    if not os.path.exists('words.json'):
        print("words.json not found")
        return

    print("Loading words.json...")
    with open('words.json', 'r', encoding='utf-8') as f:
        data = json.load(f)
    
    # 1. 收集所有二字词及其对应的拼音
    # 使用 set 提高查找效率
    two_char_dict = set()
    four_char_entries = []

    for pinyin, entries in data.items():
        for entry in entries:
            char = entry.get('char', '')
            length = len(char)
            if length == 2:
                two_char_dict.add((char, pinyin))
            elif length == 4:
                four_char_entries.append((pinyin, char))

    print(f"Collected {len(two_char_dict)} two-char word-pinyin pairs.")
    print(f"Processing {len(four_char_entries)} four-char words...")

    real_4char = []
    fake_4char = []

    # 2. 识别真假四字词
    for pinyin, char in four_char_entries:
        c1c2 = char[:2]
        c3c4 = char[2:]
        
        is_fake = False
        # 尝试切分拼音
        # 拼音长度通常大于等于4（如 'aaaa'），切分点 i 从 1 到 len-1
        for i in range(1, len(pinyin)):
            p_left = pinyin[:i]
            p_right = pinyin[i:]
            
            if (c1c2, p_left) in two_char_dict and (c3c4, p_right) in two_char_dict:
                is_fake = True
                break
        
        line = f"{pinyin} {char}"
        if is_fake:
            fake_4char.append(line)
        else:
            real_4char.append(line)

    # 3. 写入文件
    # 保持唯一性并排序（可选，按拼音排序更整洁）
    real_4char = sorted(list(set(real_4char)))
    fake_4char = sorted(list(set(fake_4char)))

    with open('real_4char.txt', 'w', encoding='utf-8') as f:
        f.write('\n'.join(real_4char))
    
    with open('fake_4char.txt', 'w', encoding='utf-8') as f:
        f.write('\n'.join(fake_4char))

    print(f"Finished!")
    print(f"Real 4-char words: {len(real_4char)}")
    print(f"Fake 4-char words: {len(fake_4char)}")

if __name__ == '__main__':
    main()
