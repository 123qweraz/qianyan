import json
import re

def strip_html(text):
    return re.sub(r'<[^>]+>', '', text)

def convert(input_file, output_file):
    entries = {}
    with open(input_file, 'r', encoding='utf-8') as f:
        for line in f:
            if ' ⬄ ' not in line:
                continue
            parts = line.split(' ⬄ ')
            if len(parts) < 2:
                continue
            
            keys_raw = parts[0].strip()
            val = strip_html(parts[1].strip())
            
            # Split variants like "24-7, 24/7"
            keys = [k.strip() for k in keys_raw.split(',')]
            
            for k in keys:
                if not k:
                    continue
                k_lower = k.lower()
                if k_lower not in entries:
                    entries[k_lower] = []
                entries[k_lower].append({"char": k, "en": val})

    with open(output_file, 'w', encoding='utf-8') as f:
        json.dump(entries, f, ensure_ascii=False, indent=2)

if __name__ == "__main__":
    convert("dicts/english/英汉大词典_del_ipa_edited.txt", "dicts/english/full_en.json")
