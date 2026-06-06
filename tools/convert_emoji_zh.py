import json
import re
import os

def load_pinyin_map(chars_file):
    """Load character to pinyin mapping from the project's own reference dictionary."""
    pinyin_map = {}
    if not os.path.exists(chars_file):
        print(f"Warning: {chars_file} not found. Will not be able to generate pinyin.")
        return pinyin_map
        
    with open(chars_file, 'r', encoding='utf-8') as f:
        for line in f:
            parts = line.strip().split('\t')
            if len(parts) >= 2:
                pinyin, char = parts[0], parts[1]
                pinyin_map[char] = pinyin
    return pinyin_map

def convert():
    # Paths relative to project root
    chars_file = 'refe_rdict/chars.txt'
    input_file = 'dicts/chinese/words/emoji_zh.txt'
    output_file = 'dicts/chinese/words/emoji_zh.json'
    
    print(f"Loading pinyin map from {chars_file}...")
    pinyin_map = load_pinyin_map(chars_file)
    
    if not os.path.exists(input_file):
        print(f"Error: {input_file} not found.")
        return
        
    emoji_dict = {}
    count = 0
    missing_pinyin_chars = set()
    
    with open(input_file, 'r', encoding='utf-8') as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            
            # Match "EMOJI (NAME)"
            match = re.search(r'^(.*?)\s+\((.*?)\)$', line)
            if match:
                emoji = match.group(1).strip()
                name = match.group(2).strip()
                
                # Convert name to pinyin
                pinyins = []
                valid_name = True
                for char in name:
                    if char in pinyin_map:
                        pinyins.append(pinyin_map[char])
                    elif '\u4e00' <= char <= '\u9fff':
                        missing_pinyin_chars.add(char)
                        pinyins.append('?') 
                        valid_name = False
                    else:
                        pinyins.append(char.lower())
                
                py_key = "".join(pinyins).replace(" ", "")
                
                if py_key not in emoji_dict:
                    emoji_dict[py_key] = []
                
                entry = {
                    "char": emoji,
                    "trad": emoji,
                    "en": name,
                    "category": "emoji",
                    "weight": 500
                }
                
                if not any(e["char"] == emoji for e in emoji_dict[py_key]):
                    emoji_dict[py_key].append(entry)
                    count += 1
            else:
                pass # Silently skip malformed lines
    
    if missing_pinyin_chars:
        print(f"Warning: {len(missing_pinyin_chars)} characters missing pinyin: {''.join(list(missing_pinyin_chars)[:20])}...")
    
    with open(output_file, 'w', encoding='utf-8') as f:
        json.dump(emoji_dict, f, ensure_ascii=False, indent=2)
    
    print(f"Successfully converted {count} emojis to {output_file}")

if __name__ == "__main__":
    convert()
