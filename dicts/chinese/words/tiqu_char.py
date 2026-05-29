import json

def extract_chars(obj, result):
    if isinstance(obj, dict):
        for k, v in obj.items():
            if k == "char":
                result.append(v)
            else:
                extract_chars(v, result)

    elif isinstance(obj, list):
        for item in obj:
            extract_chars(item, result)


def main():
    with open("low_freq.json", "r", encoding="utf-8") as f:
        data = json.load(f)

    chars = []
    extract_chars(data, chars)

    # 去重（保持顺序）
    chars = list(dict.fromkeys(chars))

    # 保存成 tongming.txt
    with open("tongming.txt", "w", encoding="utf-8") as f:
        f.write("\n".join(chars))


if __name__ == "__main__":
    main()
