import json
import requests
from stroke_parser import get_stroke_type

def test():
    # 常用字预期笔画: 1横, 2竖, 3撇, 4捺, 5折
    # 我们用 unicode 避开终端乱码
    test_chars = {
        "\u4e00": [1, 1], # 一
        "\u4e8c": [1, 1], # 二
        "\u5341": [1, 2], # 十
        "\u4eba": [3, 4], # 人
        "\u53e3": [2, 5], # 口
    }
    
    url = "https://raw.githubusercontent.com/skishore/makemeahanzi/master/graphics.txt"
    print("Fetching graphics...")
    response = requests.get(url, stream=True)

    for line in response.iter_lines():
        if not line: continue
        data = json.loads(line.decode('utf-8'))
        char = data['character']
        
        if char in test_chars:
            medians = data.get('medians', [])
            if not medians: continue
            
            s1 = get_stroke_type(medians[0])
            s2 = get_stroke_type(medians[1]) if len(medians) >= 2 else 1
            
            # 打印调试点，看看到底是什么走向
            p1 = medians[0]
            dx1 = p1[-1][0] - p1[0][0]
            dy1 = p1[-1][1] - p1[0][1]
            
            print(f"Char: {char} ({hex(ord(char))})")
            print(f"  Stroke 1: dx={dx1}, dy={dy1}, Type={s1}")
            if len(medians) >= 2:
                p2 = medians[1]
                dx2 = p2[-1][0] - p2[0][0]
                dy2 = p2[-1][1] - p2[0][1]
                print(f"  Stroke 2: dx={dx2}, dy={dy2}, Type={s2}")
            
            expect = test_chars[char]
            idx = (s1 - 1) * 5 + (s2 - 1)
            code = chr(ord('a') + idx)
            print(f"  Result: Got ({s1}, {s2}) -> {code}, Expect {expect}")

if __name__ == "__main__":
    test()
