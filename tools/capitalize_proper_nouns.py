import os

def process_pinyin(pinyin_str, mode="capitalize"):
    # 删除所有空格
    joined = pinyin_str.replace(" ", "")
    if mode == "capitalize":
        return joined.capitalize()
    else:
        return joined.lower()

def process_directory(base_path, mode="capitalize"):
    if not os.path.exists(base_path):
        print(f"跳过不存在的目录: {base_path}")
        return

    print(f"正在处理目录: {base_path} (模式: {mode})")
    for root, dirs, files in os.walk(base_path):
        for file in files:
            if file.endswith('.txt'):
                file_path = os.path.join(root, file)
                
                new_lines = []
                with open(file_path, 'r', encoding='utf-8') as f:
                    for line in f:
                        if not line.strip() or line.startswith('#'):
                            new_lines.append(line)
                            continue
                        
                        parts = line.split('\t')
                        if len(parts) >= 2:
                            word = parts[0]
                            pinyin = parts[1].strip()
                            rest = parts[2:]
                            
                            new_pinyin = process_pinyin(pinyin, mode)
                            
                            new_line = f"{word}\t{new_pinyin}"
                            for r in rest:
                                new_line += f"\t{r.strip()}"
                            new_line += "\n"
                            new_lines.append(new_line)
                        else:
                            new_lines.append(line)
                
                with open(file_path, 'w', encoding='utf-8') as f:
                    f.writelines(new_lines)

if __name__ == "__main__":
    # 1. 处理专有名词 (去空格 + 首字母大写)
    process_directory("dicts/chinese/words/capital_word", mode="capitalize")
    
    # 2. 处理普通词库 (仅去空格)
    process_directory("dicts/chinese/words/readable_lexicon", mode="lowercase")
    
    print("\n处理完成！")