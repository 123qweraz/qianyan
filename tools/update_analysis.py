import json
from collections import Counter
from pathlib import Path

def analyze():
    d = Path('dicts/chinese/chars')
    seen = set()
    p1 = Counter()
    p2 = Counter()
    p3 = Counter()
    over = Counter()
    amap = {}
    total = 0
    
    for f in ['level1.json', 'level2.json', 'level3.json']:
        path = d / f
        if not path.exists():
            continue
        data = json.loads(path.read_text(encoding='utf-8'))
        for pinyin, entries in data.items():
            for e in entries:
                char = e.get('char')
                if not char or char in seen:
                    continue
                seen.add(char)
                total += 1
                
                a = e.get('stroke_aux', '')
                if len(a) >= 1: p1[a[0]] += 1
                if len(a) >= 2: p2[a[1]] += 1
                if len(a) >= 3: p3[a[2]] += 1
                for c in a:
                    over[c] += 1
                
                if a not in amap:
                    amap[a] = []
                amap[a].append(char)

    out = ['# stroke_aux (笔画辅助码) 频率分析报告 (新规则)', f'\n基于一级、二级、三级字库中 {total} 个唯一汉字。', '\n## 1. 核心发现']
    
    t1 = p1.most_common(1)[0]
    t2 = p2.most_common(1)[0]
    t3 = p3.most_common(1)[0]
    cg = [v for v in amap.values() if len(v) > 1]
    
    out.append(f'- **首位高度集中**：字母 `{t1[0]}` ({t1[1]/sum(p1.values())*100:.2f}%) 出现频率最高。')
    out.append(f'- **第二位分布**：字母 `{t2[0]}` ({t2[1]/sum(p2.values())*100:.2f}%) 是次高频位。')
    out.append(f'- **重码分析**：共有 {len(cg)} 组重码，涉及 {sum(len(g) for g in cg)} 个汉字。最大重码组包含 {max(len(v) for v in amap.values())} 个汉字。')

    out.append('\n## 2. 位置频率统计 (Top 10)\n\n| 排名 | 首位字母 (1st) | 频率 | 第二位字母 (2nd) | 频率 | 第三位字母 (3rd) | 频率 |')
    out.append('| :--- | :--- | :--- | :--- | :--- | :--- | :--- |')
    
    p1l = p1.most_common(10)
    p2l = p2.most_common(10)
    p3l = p3.most_common(10)
    
    for i in range(10):
        c1, n1 = p1l[i] if i < len(p1l) else ('-', 0)
        c2, n2 = p2l[i] if i < len(p2l) else ('-', 0)
        c3, n3 = p3l[i] if i < len(p3l) else ('-', 0)
        s1 = sum(p1.values())
        s2 = sum(p2.values())
        s3 = sum(p3.values())
        f1 = n1/s1*100 if s1 > 0 else 0
        f2 = n2/s2*100 if s2 > 0 else 0
        f3 = n3/s3*100 if s3 > 0 else 0
        out.append(f'| {i+1} | **{c1}** | {f1:.2f}% | **{c2}** | {f2:.2f}% | **{c3}** | {f3:.2f}% |')

    out.append('\n## 3. 整体字母频率 (Overall)\n\n| 字母 | 出现次数 | 百分比 |\n| :--- | :--- | :--- |')
    ta = sum(over.values())
    for c, n in over.most_common():
        out.append(f'| {c} | {n} | {n/ta*100:.2f}% |')

    out.append('\n## 4. 重码详情 (Top 5)')
    for code, chars in sorted(amap.items(), key=lambda x: len(x[1]), reverse=True)[:5]:
        out.append(f'- `{code}`: {" ".join(chars)}')

    Path('dicts/chinese/chars/stroke_aux_analysis.md').write_text('\n'.join(out) + '\n', encoding='utf-8')

if __name__ == "__main__":
    analyze()
