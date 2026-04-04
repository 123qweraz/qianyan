import json
import os
import re

def extract_to_json(input_path, output_path, min_weight=1000):
    """
    解析 Rime 词典并转换为 JSON 格式
    格式: {"pinyin": ["word1", "word2"]}
    """
    result = {}
    count = 0
    
    print(f"Reading from {input_path}...")
    
    with open(input_path, 'r', encoding='utf-8') as f:
        for line in f:
            line = line.strip()
            # 跳过注释和空行
            if not line or line.startswith('#'):
                continue
            
            parts = line.split('	')
            if len(parts) < 2:
                continue
                
            word = parts[0]
            # Rime 拼音通常带空格，我们需要处理它
            pinyin = parts[1].replace(' ', '')
            weight = int(parts[2]) if len(parts) > 2 else 0
            
            # 过滤高频词
            if weight >= min_weight:
                if pinyin not in result:
                    result[pinyin] = []
                # 按照权重排序的需求，我们暂时存入 (word, weight)
                result[pinyin].append((word, weight))
                count += 1

    # 处理排序：同一拼音下，权重高的排前面
    final_dict = {}
    for py, words_with_weight in result.items():
        # 按权重降序排列
        sorted_words = [w[0] for w in sorted(words_with_weight, key=lambda x: x[1], reverse=True)]
        final_dict[py] = sorted_words

    print(f"Extraction complete. Found {count} high-frequency words.")
    
    with open(output_path, 'w', encoding='utf-8') as f:
        json.dump(final_dict, f, ensure_ascii=False, indent=2)
    
    print(f"Saved to {output_path}")

if __name__ == "__main__":
    input_file = "dicts/chinese/base.dict.yaml"
    output_file = "dicts/chinese/high_freq_words.json"
    # 你可以调整 min_weight。1000 是一个比较保守的常用词标准
    extract_to_json(input_file, output_file, min_weight=1000)
