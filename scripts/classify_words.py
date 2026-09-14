#!/usr/bin/env python3
"""
整理 words/high_freq.json 与 words/low_freq.json：

1. 删除 stroke_aux、trad 两个冗余字段
2. 将 rime_new 重新分类：
   - 学科专有词 → 对应学科（沿用现有学科名，文件为 high→1 / low→2）
   - 其余日常词 → 按 weight 分档：>=5000 basic1 / >=800 common1 / 其余 common2

学科识别用两条信号（命中其一即归入）：
   a) 词内中文标记词（高精度，宁缺毋滥）
   b) 英文注释 en 关键词
识别不到的进入日常词档。

仅改写 category 值，保留原有字段顺序；输出结构化统计与抽查样例。
"""

import json
import random
from collections import Counter

FILES = [
    'dicts/chinese/words/high_freq.json',
    'dicts/chinese/words/low_freq.json',
]

TIER = [('basic1', 5000), ('common1', 800), ('common2', 0)]

# (学科, 中文标记词...) —— 标记词命中即判为该学科
CHINESE_MARKERS = [
    ('chemistry', ['元素周期', '化合价', '氧化反应', '还原反应', '催化剂', '化学性质',
                   '化学反应', '化合物', '有机物', '无机物', '电解质', '分子式', '氢氧',
                   '氯化', '氧化物', '常温下', '化学式', '酸根', '碱式', '灼烧', '焰色',
                   '稀盐酸', '浓硫酸', '氢氧化', '碳酸钙', '苏打', '小苏打']),
    ('mathematics', ['方程', '函数', '几何', '定理', '公式', '概率', '数列', '向量',
                     '矩阵', '不等式', '代数', '积分', '微分', '正比例', '反比例',
                     '一次方程', '二次根', '平方根', '算术', '代数式', '多项式', '因数',
                     '公约数', '公倍数', '中位数', '平均数', '倒数', '分数值', '习题',
                     '通分', '约分', '化简', '解方程', '方程组']),
    ('physics', ['物理', '力学', '动能', '势能', '加速度', '重力', '摩擦力', '惯性',
                 '磁场', '电场', '电压', '电流表', '电阻', '电源', '导线', '电路',
                 '串联', '并联', '电流', '功率', '能量守恒', '光速', '波长', '频率',
                 '共振', '量子', '粒子', '原子核', '相对论', '电磁', '声速', '压强',
                 '浮力', '杠杆', '滑轮', '做功', '热能', '机械能', '电动机']),
    ('biology', ['生物学', '基因', '细胞', '遗传', '进化', '细菌', '病毒', '免疫',
                 '酶', '激素', '神经', '血液循环', '光合作用', '蛋白质', '氨基酸',
                 '染色体', '变异', '种群', '生态系统', '细胞核', '细胞质', '线粒体',
                 '叶绿体', '遗传学', '生物圈', '新陈代谢', '有机物代谢', '水循环',
                 '食物链', '组织器官', '生物体', '呼吸道', '消化道', '受精卵']),
    ('history', ['历史', '朝代', '王朝', '皇帝', '革命', '起义', '条约', '帝国',
                 '封建', '遗址', '文物', '考古', '青铜器', '甲骨文', '长城', '故宫',
                 '抗日战争', '红军', '长征', '解放战争', '鸦片战争', '洋务运动',
                 '文化大革命', '新石器', '旧石器', '史记', '春秋时期', '战国时期',
                 '秦始皇', '汉武帝', '唐太宗', '康熙', '贞观', '开元']),
    ('geography', ['地理', '山脉', '河流', '湖泊', '高原', '平原', '盆地', '沙漠',
                   '海峡', '半岛', '岛屿', '气候', '省会', '直辖市', '自治区', '气象',
                   '地形', '经纬', '纬度', '经度', '时区', '板块', '地震', '火山',
                   '季风', '降水', '土壤', '森林资源', '矿产资源', '长江流域',
                   '黄河流域', '珠穆朗玛', '长江三角洲']),
    ('politics', ['政治', '政府', '制度', '主义', '选举', '议会', '总统', '宪法',
                  '法律', '法规', '司法', '外交', '联邦', '共和国', '民主', '专政',
                  '政权', '阶级', '社会主义', '资本主义', '共产党', '全国人大',
                  '人民代表大会', '政策制定', '公务员', '检察院', '法院']),
    ('information_technology', ['计算机', '电脑', '网络', '软件', '硬件', '数据',
                                '互联网', '编程', '代码', '算法', '人工智能', '数据库',
                                '服务器', '芯片', '操作系统', '内存', '硬盘', '显示屏',
                                '键盘', '鼠标', '云计算', '大数据', '物联网', '软件工程',
                                '网址', '网页', '网站', '用户界面', '文件系统','处理器']),
    ('english', ['英语', '英文', '单词', '语法', '口语', '听力', '翻译', '英语角',
                 '英文名', '英语音标', '英语单词', '英语语法', '英语口语', '英语听力',
                 '英语水平', '英汉', '汉英', '雅思', '托福', '四级', '六级']),
    ('music', ['音乐', '歌曲', '歌手', '乐队', '旋律', '节奏', '钢琴', '吉他', '交响',
               '乐章', '专辑', '民谣', '摇滚', '古典音乐', '流行音乐', '乐谱', '作曲',
               '演奏', '演唱', '琴键', '音箱', '乐器', '音响']),
    ('arts', ['美术', '艺术', '绘画', '雕塑', '设计', '书法', '展览', '美术馆',
              '画廊', '画作', '水墨', '油画', '素描', '国画', '建筑艺术', '色彩',
              '构图', '审美', '艺术品', '剪刀艺术']),
    ('pe_and_health', ['体育', '运动', '足球', '篮球', '羽毛球', '网球', '体操', '田径',
                       '游泳', '球赛', '锻炼', '健身', '比赛', '跑步', '跳远', '跳高',
                       '铅球', '排球', '乒乓', '健康生活', '身体素质', '减肥',
                       '运动员', '教练员', '裁判', '奥运', '亚运']),
    ('general_technology', ['技术', '工程', '科技', '机械', '工业', '制造', '电路',
                            '机电', '自动化', '焊接', '锻造', '零件', '机床', '生产',
                            '工艺', '材料学', '设计图纸', '规模化生产']),
    ('foreign_languages', ['外语', '日语', '法语', '德语', '俄语', '西班牙语', '韩语',
                           '阿拉伯语', '意大利语', '葡萄牙语', '泰语', '越南语',
                           '外文', '外语学习']),
]

# (学科, 英文注释关键词...) —— en 小写匹配
ENGLISH_MARKERS = [
    ('chemistry', ['chemistry', 'chemical', 'element', 'acid', 'base', 'reaction',
                   'oxidation', 'molecule', 'compound', 'equation', 'solution',
                   'sodium', 'chloride', 'hydrogen', 'oxygen', 'nitrogen', 'carbon',
                   'biology (?)', 'periodic']),
    ('mathematics', ['math', 'mathematical', 'equation', 'function', 'geometry',
                     'theorem', 'algebra', 'probability', 'integer', 'geometry',
                     'triangle', 'angle', 'fraction', 'formula', 'calculus',
                     'derivative', 'integral', 'matrix', 'vector']),
    ('physics', ['physics', 'force', 'energy', 'gravity', 'velocity', 'acceleration',
                 'electric', 'magnet', 'current', 'voltage', 'resistance', 'circuit',
                 'frequency', 'wavelength', 'quantum', 'particle', 'momentum',
                 'thermo', 'friction', 'pressure', 'inertia']),
    ('biology', ['biology', 'gene', 'genetic', 'cell', 'organism', 'bacteria',
                 'virus', 'immune', 'enzyme', 'hormone', 'neuron', 'evolution',
                 'protein', 'amino', 'dna', 'rna', 'photosynthesis', 'ecosystem',
                 'species', 'disease', 'anatomy']),
    ('history', ['history', 'historical', 'dynasty', 'emperor', 'revolution', 'war',
                 'treaty', 'empire', 'ancient', 'archae', 'heritage', 'long march',
                 'kmt', 'civil war', 'opium']),
    ('geography', ['geograph', 'mountain', 'river', 'lake', 'plateau', 'plain',
                   'basin', 'desert', 'strait', 'peninsula', 'island', 'climate',
                   'province', 'municipality', 'region', 'latitude', 'longitude',
                   'continent', 'ocean', 'valley', 'terrain']),
    ('politics', ['politic', 'government', 'policy', 'election', 'parliament',
                  'president', 'constitution', 'law', 'legal', 'judicial',
                  'diplomat', 'federal', 'republic', 'democracy', 'socialist',
                  'communist', 'court', 'legislative']),
    ('information_technology', ['computer', 'software', 'hardware', 'network',
                                'internet', 'data', 'program', 'code', 'algorithm',
                                'database', 'server', 'artificial intelligence',
                                'chip', 'system', 'memory', 'keyboard', 'website',
                                'web ', 'technology ic', 'interface']),
    ('english', ['english', 'vocabulary', 'pronunciation', 'translation', 'grammar',
                 'speaking', 'listening', 'reading', 'toefl', 'ielts', 'english-']),
    ('music', ['music', 'song', 'singer', 'band', 'melody', 'rhythm', 'piano',
               'guitar', 'symphony', 'album', 'folk', 'rock', 'compos', 'orchestra',
               'instrument', 'concert', 'tune']),
    ('arts', ['art', 'painting', 'sculpture', 'design', 'calligraphy', 'exhibition',
              'gallery', 'drawing', 'sketch', 'watercolor', 'aesthetic', 'artwork',
              'portrait', 'canvas']),
    ('pe_and_health', ['sport', 'football', 'basketball', 'badminton', 'tennis',
                       'gymnastic', 'athletics', 'swim', 'running', 'jump', 'shot put',
                       'volleyball', 'table tennis', 'fitness', 'exercise', 'health',
                       'coach', 'athlete', 'olympic', 'marathon', 'yoga']),
    ('general_technology', ['technology', 'engineering', 'machine', 'industry',
                            'manufactur', 'circuit', 'automation', 'weld', 'machinery',
                            'process', 'material', 'production']),
    ('foreign_languages', ['japanese', 'french', 'german', 'russian', 'spanish',
                           'korean', 'arabic', 'italian', 'portuguese', 'thai',
                           'vietnamese', 'foreign language']),
]


def classify(word, en):
    """返回学科名或 None。"""
    for subj, markers in CHINESE_MARKERS:
        for m in markers:
            if m in word:
                return subj
    if en:
        en_l = en.lower()
        # 词很长且 en 标注为姓氏/名字时跳过命名干扰
        for subj, markers in ENGLISH_MARKERS:
            for m in markers:
                if m in en_l:
                    return subj
    return None


def tier(weight):
    for name, thr in TIER:
        if weight >= thr:
            return name
    return TIER[-1][0]


def main():
    random.seed(42)
    for path in FILES:
        is_high = 'high' in path
        with open(path, encoding='utf-8') as f:
            data = json.load(f)

        stats = Counter()
        samples = {}
        changed = 0
        for pinyin, entries in data.items():
            for e in entries:
                # 删除冗余字段
                if 'stroke_aux' in e:
                    del e['stroke_aux']
                if 'trad' in e:
                    del e['trad']
                if e.get('category') == 'rime_new':
                    word = e['char']
                    en = e.get('en', '')
                    subj = classify(word, en)
                    new_cat = (f'{subj}1' if is_high else f'{subj}2') if subj else tier(e.get('weight', 0))
                    e['category'] = new_cat
                    changed += 1
                    stats[new_cat] += 1
                    samples.setdefault(new_cat, []).append((word, e.get('weight', 0), en))

        with open(path, 'w', encoding='utf-8') as f:
            json.dump(data, f, ensure_ascii=False, indent=2)
            f.write('\n')

        print(f'==== {path}')
        print(f'  重分类 {changed} 条 rime_new')
        print('  分布:')
        for cat, n in stats.most_common():
            print(f'      {cat:<24} {n:>7}')
        print('  抽查样例（每类至多 10 条）:')
        for cat, n in sorted(stats.items()):
            if n == 0:
                continue
            r = random.sample(samples[cat], min(10, len(samples[cat])))
            line = ' | '.join(f'{w}({w_} {en[:14]})' for w, w_, en in r)
            print(f'      [{cat}] {line}')


if __name__ == '__main__':
    main()