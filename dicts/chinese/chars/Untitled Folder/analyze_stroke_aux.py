import json
from collections import Counter

def analyze():
    try:
        with open('chars.json', 'r', encoding='utf-8') as f:
            data = json.load(f)
    except FileNotFoundError:
        print("Error: chars.json not found.")
        return
    except json.JSONDecodeError:
        print("Error: Failed to decode chars.json.")
        return

    pos1 = Counter()
    pos2 = Counter()
    pos3 = Counter()
    overall = Counter()
    
    seen_chars = set()
    total_entries = 0
    unique_chars_processed = 0

    # The structure is { "pinyin": [ { "char": "...", "stroke_aux": "..." }, ... ] }
    for pinyin, char_list in data.items():
        for item in char_list:
            char_name = item.get('char')
            if not char_name:
                continue
            
            # To avoid over-counting polyphones, we only count each unique character once
            if char_name in seen_chars:
                continue
            seen_chars.add(char_name)
            unique_chars_processed += 1
            
            stroke_aux = item.get('stroke_aux', '')
            if not stroke_aux:
                continue
            
            # Position 1
            if len(stroke_aux) >= 1:
                pos1[stroke_aux[0]] += 1
            # Position 2
            if len(stroke_aux) >= 2:
                pos2[stroke_aux[1]] += 1
            # Position 3
            if len(stroke_aux) >= 3:
                pos3[stroke_aux[2]] += 1
            
            # Overall
            for char in stroke_aux:
                overall[char] += 1

    def print_counter(name, counter):
        print(f"=== {name} ===")
        # Sort by count descending, then by character ascending
        sorted_items = sorted(counter.items(), key=lambda x: (-x[1], x[0]))
        total = sum(counter.values())
        print(f"Total count: {total}")
        for char, count in sorted_items:
            percentage = (count / total * 100) if total > 0 else 0
            print(f"{char}: {count:5} ({percentage:6.2f}%)")
        print()

    print(f"Total unique characters processed: {unique_chars_processed}\n")
    print_counter("1st Letter Frequency (shou zishou)", pos1)
    print_counter("2nd Letter Frequency (dierge zimu)", pos2)
    print_counter("3rd Letter Frequency (disange zimu)", pos3)
    print_counter("Overall Letter Frequency (zhengge bihua puzhuma)", overall)

if __name__ == "__main__":
    analyze()
