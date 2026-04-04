import os
import re
import json

def get_stroke_mapping():
    # 5x5 矩阵映射
    mapping = {
        '11': 'g', '12': 'f', '13': 'd', '14': 's', '15': 'a',
        '21': 'h', '22': 'j', '23': 'k', '24': 'l', '25': 'm',
        '31': 't', '32': 'r', '33': 'e', '34': 'w', '35': 'q',
        '41': 'y', '42': 'u', '43': 'i', '44': 'o', '45': 'p',
        '51': 'n', '52': 'b', '53': 'v', '54': 'c', '55': 'x'
    }
    # 奇数笔画末尾单笔映射 (各区首字母)
    single_mapping = {'1': 'g', '2': 'h', '3': 't', '4': 'y', '5': 'n'}
    return mapping, single_mapping

def encode_stroke_sequence(stroke_seq, mapping, single_mapping):
    if not stroke_seq:
        return ""
    res = ""
    for i in range(0, len(stroke_seq) - 1, 2):
        pair = stroke_seq[i:i+2]
        res += mapping.get(pair, '')
    if len(stroke_seq) % 2 != 0:
        res += single_mapping.get(stroke_seq[-1], '')
    return res

def parse_js_data(file_path):
    stroke_dict = {}
    total_raw_records = 0
    try:
        with open(file_path, 'r', encoding='utf-8') as f:
            content = f.read()
            # 改进正则：支持单/双引号，支持空格
            matches = re.findall(r'\[\s*["\'](.*?)["\']\s*,\s*["\'](.*?)["\']\s*\]', content)
            total_raw_records = len(matches)
            for char, strokes in matches:
                # 过滤掉非数字的笔画数据
                clean_strokes = "".join(filter(str.isdigit, strokes))
                stroke_dict[char] = clean_strokes
    except Exception as e:
        print(f"Error parsing JS file: {e}")
    return stroke_dict, total_raw_records

def main():
    mapping, single_mapping = get_stroke_mapping()
    js_path = os.path.join('reference', 'bh', 'data_v2.js')
    
    print(f"正在读取原始笔画数据: {js_path}")
    char_to_strokes, total_raw = parse_js_data(js_path)
    print(f"原始 JS 记录数: {total_raw}")
    
    if not char_to_strokes:
        print("未找到有效数据，请检查路径。")
        return

    # 1. 生成单字词典
    print("正在编码单字...")
    encoded_words = {}
    all_syllables = set()
    total_chars_processed = 0
    
    for char, strokes in char_to_strokes.items():
        code = encode_stroke_sequence(strokes, mapping, single_mapping)
        if code:
            total_chars_processed += 1
            if code not in encoded_words:
                encoded_words[code] = []
            encoded_words[code].append(char)
            all_syllables.add(code)

    print(f"处理完成的汉字总数: {total_chars_processed}")
    print(f"产生的唯一编码数: {len(encoded_words)}")
    print(f"重码（多个字共用编码）的情况: {total_chars_processed - len(encoded_words)} 处")

    # 2. 尝试读取词组并编码 (全码拼接)
    # 这里我们先处理单字，词组逻辑可以在 level2 进一步扩展
    # 为了演示，我们先生成基础词典
    
    output_dir = os.path.join('dicts', 'stroke')
    words_dir = os.path.join(output_dir, 'words')
    os.makedirs(words_dir, exist_ok=True)

    # 保存 syllables.txt
    with open(os.path.join(output_dir, 'syllables.txt'), 'w', encoding='utf-8') as f:
        for s in sorted(list(all_syllables)):
            f.write(f"{s}\n")

    # 保存 stroke_char.json
    with open(os.path.join(words_dir, 'stroke_char.json'), 'w', encoding='utf-8') as f:
        json.dump(encoded_words, f, ensure_ascii=False, indent=2)

    print(f"转换完成！已生成词典到 {output_dir}")
    print(f"总计单字编码: {len(encoded_words)}")

if __name__ == "__main__":
    main()
