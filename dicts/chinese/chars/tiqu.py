import json
from collections import defaultdict

def load_json(path):
    with open(path, "r", encoding="utf-8") as f:
        return json.load(f)

def extract_pairs(data):
    results = []

    # 按 key（a / ai / ...）处理
    for key, items in data.items():
        letter_groups = defaultdict(list)

        # 按英文首字母分组
        for item in items:
            en = item.get("en", "")
            if not en:
                continue

            first_letter = en[0].lower()
            letter_groups[first_letter].append(item)

        # 只保留 >=2 的组
        for group in letter_groups.values():
            if len(group) >= 2:
                for item in group:
                    results.append(f"{key} {item['char']} {item['en']}")

    return results

def main():
    data = load_json("level3.json")
    results = extract_pairs(data)

    with open("level3_output.txt", "w", encoding="utf-8") as f:
        f.write("\n".join(results))

if __name__ == "__main__":
    main()
