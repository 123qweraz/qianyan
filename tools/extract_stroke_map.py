import json

def extract():
    with open('dicts/shengpizi/stroke_chars.json', 'r', encoding='utf-8') as f:
        data = json.load(f)
    
    char_map = {}
    for key, items in data.items():
        for item in items:
            char = item.get('char')
            if char:
                # If there are multiple keys for one char, we might need a strategy.
                # For now, we take the first one we find.
                if char not in char_map:
                    char_map[char] = key

    with open('stroke_map.txt', 'w', encoding='utf-8') as f:
        for char, key in char_map.items():
            f.write(f"{char} {key}\n")

if __name__ == "__main__":
    extract()
