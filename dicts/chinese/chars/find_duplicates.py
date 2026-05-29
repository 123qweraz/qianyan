import json
import collections

def find_duplicates(input_file, output_file):
    try:
        with open(input_file, 'r', encoding='utf-8') as f:
            data = json.load(f)
    except Exception as e:
        print(f"Error reading {input_file}: {e}")
        return

    results = []

    for key, chars in data.items():
        # Group by the first letter of the English translation
        first_letter_map = collections.defaultdict(list)
        for char_info in chars:
            en = char_info.get('en', '')
            if en:
                # Take the first letter and normalize to uppercase
                first_letter = en[0].upper()
                first_letter_map[first_letter].append(char_info)
        
        # Check for groups with more than one character
        for first_letter, group in first_letter_map.items():
            if len(group) > 1:
                for char_info in group:
                    results.append(f"{key} {char_info['char']} {char_info['en']}")

    try:
        with open(output_file, 'w', encoding='utf-8') as f:
            f.write('\n'.join(results))
        print(f"Results saved to {output_file}")
    except Exception as e:
        print(f"Error writing to {output_file}: {e}")

if __name__ == "__main__":
    find_duplicates('chars.json', 'duplicates.txt')
