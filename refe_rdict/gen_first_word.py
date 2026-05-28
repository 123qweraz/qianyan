import json
import os

def main():
    if not os.path.exists('words.json'):
        print("words.json not found")
        return

    with open('words.json', 'r', encoding='utf-8') as f:
        data = json.load(f)
    
    output_lines = []
    
    # Sort pinyin keys for consistent output
    sorted_pinyins = sorted(data.keys())
    
    for pinyin in sorted_pinyins:
        entries = data[pinyin]
        # Find the FIRST two-character word in the list (usually the one with highest weight)
        first_two_char = None
        for entry in entries:
            char = entry.get('char', '')
            if len(char) == 2:
                first_two_char = char
                break
        
        if first_two_char:
            output_lines.append(f"{pinyin} {first_two_char}")
    
    with open('words_first_2char.txt', 'w', encoding='utf-8') as f:
        f.write('\n'.join(output_lines))
    print(f"Generated words_first_2char.txt with {len(output_lines)} entries.")

if __name__ == '__main__':
    main()
