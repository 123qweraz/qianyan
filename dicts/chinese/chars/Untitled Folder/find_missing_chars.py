import json
import re

def get_chars_from_bihua(file_path):
    chars = set()
    try:
        with open(file_path, 'r', encoding='utf-8') as f:
            for line in f:
                parts = line.strip().split('\t')
                if parts:
                    chars.add(parts[0])
    except Exception as e:
        print(f"Error reading {file_path}: {e}")
    return chars

def get_chars_from_json(file_path):
    chars = set()
    try:
        with open(file_path, 'r', encoding='utf-8') as f:
            data = json.load(f)
            # Assuming the structure is { pinyin: [ { "char": "..." }, ... ] }
            for pinyin in data:
                for entry in data[pinyin]:
                    if 'char' in entry:
                        chars.add(entry['char'])
    except Exception as e:
        print(f"Error reading {file_path}: {e}")
    return chars

def main():
    bihua_chars = get_chars_from_bihua('bihua.txt')
    
    json_chars = set()
    json_files = ['chars.json', 'level2.json', 'level3.json']
    for json_file in json_files:
        json_chars.update(get_chars_from_json(json_file))
    
    missing_chars = bihua_chars - json_chars
    
    output_file = 'missing_chars.txt'
    with open(output_file, 'w', encoding='utf-8') as f:
        for char in sorted(list(missing_chars)):
            f.write(char + '\n')
    
    print(f"Found {len(missing_chars)} missing characters. Saved to {output_file}.")

if __name__ == "__main__":
    main()
