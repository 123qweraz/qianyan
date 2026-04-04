import json

def analyze_stroke_collisions():
    path = 'dicts/stroke/words/stroke_words.json'
    with open(path, 'r', encoding='utf-8') as f:
        data = json.load(f)
    
    total_codes = len(data)
    total_words = 0
    unique_codes = 0
    collision_counts = {}
    
    max_words_per_code = 0
    most_conflicting_code = ""
    
    for code, entries in data.items():
        count = len(entries)
        total_words += count
        
        if count == 1:
            unique_codes += 1
        else:
            if count not in collision_counts:
                collision_counts[count] = 0
            collision_counts[count] += 1
            
            if count > max_words_per_code:
                max_words_per_code = count
                most_conflicting_code = code

    print("--- 笔画 4 键组词重码率分析报告 ---")
    print(f"总词条数: {total_words}")
    print(f"总编码数: {total_codes}")
    print(f"唯一码数量: {unique_codes} ({unique_codes/total_codes*100:.2f}%)")
    print(f"平均重码数: {total_words/total_codes:.2f}")
    print(f"最大重码数: {max_words_per_code} (编码: {most_conflicting_code})")
    
    print("\n重码分布详情:")
    for count in sorted(collision_counts.keys()):
        num_codes = collision_counts[count]
        print(f"  {count} 词重码: {num_codes} 码")

if __name__ == "__main__":
    analyze_stroke_collisions()
