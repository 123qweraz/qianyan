import json
import os
import re

def find_missing_standard_chars():
    base_dir = 'dicts/chinese/chars'
    json_files = ['level1.json', 'level2.json', 'level3.json']
    # The file name has complex characters, using glob-like approach or exact name if known
    # From previous list_directory: "通规8105(字 笔画 编号 UCS 笔顺).txt"
    standard_file = 'dicts/chinese/chars/Untitled Folder/通规8105(字 笔画 编号 UCS 笔顺).txt'
    
    # 1. Load characters from JSON files
    json_chars = set()
    for f_name in json_files:
        path = os.path.join(base_dir, f_name)
        if os.path.exists(path):
            with open(path, 'r', encoding='utf-8') as f:
                data = json.load(f)
                # JSON structure: {"pinyin": [{"char": "...", ...}, ...]}
                for entries in data.values():
                    for entry in entries:
                        json_chars.add(entry['char'])
        else:
            print(f"Warning: {path} not found.")

    # 2. Load characters from the standard text file
    standard_chars = set()
    if os.path.exists(standard_file):
        with open(standard_file, 'r', encoding='utf-8') as f:
            for line in f:
                line = line.strip()
                if not line or line.startswith('#'):
                    continue
                # Expected format: "字 笔画 编号 UCS 笔顺"
                # Example: "嘿 15画 3252 0563F 251254312114444"
                parts = line.split()
                if parts:
                    char = parts[0]
                    if len(char) == 1: # Basic sanity check for a single character
                        standard_chars.add(char)
    else:
        print(f"Error: {standard_file} not found.")
        return

    # 3. Find difference
    missing = standard_chars - json_chars
    
    print(f"Total characters in JSON (level1-3): {len(json_chars)}")
    print(f"Total characters in Standard 8105 file: {len(standard_chars)}")
    print(f"Characters in Standard but MISSING from JSON: {len(missing)}")
    
    if missing:
        sorted_missing = sorted(list(missing))
        print("\nFirst 100 missing characters:")
        print("".join(sorted_missing[:100]))
        
        # Save to a file for review
        with open('missing_standard_chars.txt', 'w', encoding='utf-8') as f:
            f.write("".join(sorted_missing))
        print(f"\nAll missing characters saved to missing_standard_chars.txt")

if __name__ == "__main__":
    find_missing_standard_chars()
