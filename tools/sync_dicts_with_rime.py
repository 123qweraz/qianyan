import json
import os

def get_jianpin(pinyin_list):
    # 简单的声母提取逻辑
    # 注意：这里假设拼音是规范的全拼，提取首字母
    # 实际情况可能更复杂（如 zh, ch, sh），但简拼通常只取首字母
    res = ""
    # 这里我们简单处理：每个音节的首字母
    # 假设输入的 pinyin 已经是处理好的，如 "nihao"
    # 但由于我们是从 rime 的 "ni hao" 来的，直接处理更方便
    return "".join([part[0] for part in pinyin_list if part])

def update_all_dicts():
    rime_yaml = "dicts/chinese/base.dict.yaml"
    words_json_path = "dicts/chinese/words/words.json"
    jianpin_json_path = "dicts/chinese/words/words_jianpin.json"
    
    # 1. 加载 Rime 数据
    # word -> {pinyin_key: (raw_pinyin_list, weight)}
    rime_map = {}
    print(f"Loading Rime data from {rime_yaml}...")
    with open(rime_yaml, 'r', encoding='utf-8') as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith('#'): continue
            parts = line.split('	')
            if len(parts) >= 2:
                word = parts[0]
                raw_pinyin = parts[1]
                weight = int(parts[2]) if len(parts) > 2 else 0
                pinyin_key = raw_pinyin.replace(' ', '')
                
                if word not in rime_map or weight > rime_map[word]['weight']:
                    rime_map[word] = {
                        'key': pinyin_key,
                        'list': raw_pinyin.split(' '),
                        'weight': weight
                    }

    # 2. 读取旧的 words.json
    print(f"Reading {words_json_path}...")
    with open(words_json_path, 'r', encoding='utf-8') as f:
        old_words_data = json.load(f)

    new_words_data = {}
    
    # 3. 转换逻辑
    print("Converting words and updating pinyins...")
    for old_py, entries in old_words_data.items():
        for entry in entries:
            word = entry['char']
            if word in rime_map:
                # 使用 Rime 的读音和权重
                new_py = rime_map[word]['key']
                entry['weight'] = rime_map[word]['weight']
                # 记录原始拼音列表用于生成简拼
                entry['_pinyin_list'] = rime_map[word]['list']
            else:
                # 没找到则保留，给低权重
                new_py = old_py
                entry['weight'] = entry.get('weight', 10)
                # 尝试猜测拼音列表（由于是全拼，这种猜测不一定准，但总比没有好）
                entry['_pinyin_list'] = [old_py] 

            if new_py not in new_words_data:
                new_words_data[new_py] = []
            new_words_data[new_py].append(entry)

    # 排序：按权重降序
    for py in new_words_data:
        new_words_data[py].sort(key=lambda x: x.get('weight', 0), reverse=True)

    # 4. 生成新的简拼词库
    print("Generating new jianpin dictionary...")
    new_jianpin_data = {}
    for py, entries in new_words_data.items():
        for entry in entries:
            # 简拼通常只对 2 字及以上的词有意义
            if len(entry['_pinyin_list']) >= 1:
                jp = get_jianpin(entry['_pinyin_list'])
                if jp not in new_jianpin_data:
                    new_jianpin_data[jp] = []
                
                # 移除临时属性并存入简拼
                jp_entry = entry.copy()
                jp_entry.pop('_pinyin_list', None)
                new_jianpin_data[jp].append(jp_entry)

    # 简拼也按权重排序
    for jp in new_jianpin_data:
        new_jianpin_data[jp].sort(key=lambda x: x.get('weight', 0), reverse=True)

    # 5. 写回文件
    print("Saving files...")
    # 移除辅助属性并保存 words.json
    for py in new_words_data:
        for entry in new_words_data[py]:
            entry.pop('_pinyin_list', None)

    with open(words_json_path, 'w', encoding='utf-8') as f:
        json.dump(new_words_data, f, ensure_ascii=False, indent=2)
    
    with open(jianpin_json_path, 'w', encoding='utf-8') as f:
        json.dump(new_jianpin_data, f, ensure_ascii=False, indent=2)

    print("Success! Updated words.json and words_jianpin.json.")

if __name__ == "__main__":
    update_all_dicts()
