import json
import os

def main():
    if not os.path.exists('words.json'):
        print("words.json not found")
        return

    print("Loading words.json...")
    with open('words.json', 'r', encoding='utf-8') as f:
        data = json.load(f)
    
    # 用字典存储不同长度的首选词
    files_data = {
        '3char': [],
        '4char': [],
        'more_char': []
    }
    
    sorted_pinyins = sorted(data.keys())
    
    for pinyin in sorted_pinyins:
        entries = data[pinyin]
        
        # 记录每种长度是否已经找到了第一个词
        found = {'3': False, '4': False, 'more': False}
        
        for entry in entries:
            char = entry.get('char', '')
            length = len(char)
            
            if length == 3 and not found['3']:
                files_data['3char'].append(f"{pinyin} {char}")
                found['3'] = True
            elif length == 4 and not found['4']:
                files_data['4char'].append(f"{pinyin} {char}")
                found['4'] = True
            elif length >= 5 and not found['more']:
                files_data['more_char'].append(f"{pinyin} {char}")
                found['more'] = True
                
            # 如果所有长度都找到了，可以提前跳出当前拼音的循环
            if all(found.values()):
                break
    
    # 写入文件
    configs = [
        ('words_first_3char.txt', '3char'),
        ('words_first_4char.txt', '4char'),
        ('words_first_long.txt', 'more_char')
    ]
    
    for filename, key in configs:
        with open(filename, 'w', encoding='utf-8') as f:
            f.write('\n'.join(files_data[key]))
        print(f"Generated {filename} with {len(files_data[key])} entries.")

if __name__ == '__main__':
    main()
