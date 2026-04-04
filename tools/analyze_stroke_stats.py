import json

def analyze_stroke_stats():
    path = 'dicts/stroke/words/stroke_char.json'
    with open(path, 'r', encoding='utf-8') as f:
        data = json.load(f)
    
    # 统计表：键为编码长度，值为 [汉字数, 总权重]
    stats = {}
    total_chars = 0
    total_weight = 0
    chars_above_8 = 0
    
    for code, entries in data.items():
        L = len(code)
        # 估算笔画数：大部分是 2L，末尾可能是单笔则为 2L-1
        # 我们按编码长度分类更准确
        if L not in stats:
            stats[L] = [0, 0]
        
        for entry in entries:
            weight = entry.get('weight', 0)
            stats[L][0] += 1
            stats[L][1] += weight
            total_chars += 1
            total_weight += weight
            
            # 8 笔以上基本对应编码长度 >= 5 (9笔+) 
            # 编码长度 4 (7-8笔) 的我们也可以单独算一下
            if L >= 5:
                chars_above_8 += 1

    print("--- 汉字笔画分布与频率分析报告 ---")
    print(f"词典总字数: {total_chars}")
    print(f"8 笔以上 (编码长度 >= 5) 的汉字数: {chars_above_8} ({chars_above_8/total_chars*100:.2f}%)")
    
    print("\n分布详情 (按编码长度):")
    print(f"{'编码长度':<10} | {'估算笔画':<10} | {'汉字数量':<10} | {'频率占比 (权重)':<10}")
    print("-" * 60)
    
    for L in sorted(stats.keys()):
        count, weight = stats[L]
        freq_pct = (weight / total_weight * 100) if total_weight > 0 else 0
        stroke_range = f"{2*L-1}-{2*L}"
        print(f"{L:<10} | {stroke_range:<10} | {count:<10} | {freq_pct:>8.2f}%")

if __name__ == "__main__":
    analyze_stroke_stats()
