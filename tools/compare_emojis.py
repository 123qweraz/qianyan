import json
import os

def compare_emojis():
    file1 = 'dicts/chinese/words/emoji.json'
    file2 = 'dicts/chinese/words/emoji_zh.json'
    
    if not os.path.exists(file1) or not os.path.exists(file2):
        print(f"Error: One or both files missing: {file1}, {file2}")
        return

    with open(file1, 'r', encoding='utf-8') as f:
        data1 = json.load(f)
    with open(file2, 'r', encoding='utf-8') as f:
        data2 = json.load(f)

    # Extract all unique emoji characters from each file
    emojis1 = set()
    for entries in data1.values():
        for entry in entries:
            emojis1.add(entry['char'])

    emojis2 = set()
    for entries in data2.values():
        for entry in entries:
            emojis2.add(entry['char'])

    # Find emojis in file1 that are NOT in file2
    unique_to_file1 = emojis1 - emojis2
    
    print(f"Total emojis in {file1}: {len(emojis1)}")
    print(f"Total emojis in {file2}: {len(emojis2)}")
    print(f"Emojis in {file1} but NOT in {file2}: {len(unique_to_file1)}")
    
    if unique_to_file1:
        print("\nList of unique emojis in emoji.json:")
        for char in sorted(list(unique_to_file1)):
            # Find keywords for this emoji in file1
            keywords = []
            for kw, entries in data1.items():
                if any(e['char'] == char for e in entries):
                    keywords.append(kw)
            print(f"{char}: {','.join(keywords)}")

if __name__ == "__main__":
    compare_emojis()
