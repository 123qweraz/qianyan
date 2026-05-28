import json
import os

def get_shortest_unique_prefix(target, others):
    if not others:
        return target[0] if target else ""
    
    target_lower = target.lower()
    others_lower = [o.lower() for o in others]
    
    for i in range(1, len(target) + 1):
        prefix = target_lower[:i]
        is_unique = True
        for other in others_lower:
            if other.startswith(prefix):
                is_unique = False
                break
        if is_unique:
            return target[:i]
    return target

def main():
    if not os.path.exists('chars.json'):
        print("chars.json not found")
        return

    with open('chars.json', 'r', encoding='utf-8') as f:
        data = json.load(f)
    
    output_lines = []
    
    sorted_pinyins = sorted(data.keys())
    
    for pinyin in sorted_pinyins:
        entries = data[pinyin]
        # Filter for single-character entries
        single_char_entries = [e for e in entries if len(e.get('char', '')) == 1]
        if not single_char_entries:
            continue
            
        single_char_entries.sort(key=lambda x: x.get('weight', 0), reverse=True)
        all_en_for_pinyin = [e.get('en', '') for e in entries if e.get('en')]
        
        for entry in single_char_entries:
            char = entry.get('char', '')
            en = entry.get('en', '')
            if not en:
                continue
                
            other_en = [e for e in all_en_for_pinyin if e != en]
            prefix = get_shortest_unique_prefix(en, other_en)
            aux_code = pinyin + prefix
            
            output_lines.append(f"{pinyin}	{char}	{en}	{aux_code}")
    
    with open('chars.txt', 'w', encoding='utf-8') as f:
        f.write('\n'.join(output_lines))
    print("Generated chars.txt")

if __name__ == '__main__':
    main()
