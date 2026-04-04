import json
import requests

# 笔画映射规则: 横(1), 竖(2), 撇(3), 点/捺(4), 折(5)
# 5x5 矩阵: (s1-1)*5 + (s2-1) -> A-Y

def get_stroke_type(path):
    """
    极简笔画识别逻辑 (基于 SVG 路径走向)
    M x,y L x,y ...
    横: dx > 0, abs(dy) < dx
    竖: dy > 0, abs(dx) < dy
    撇: dx < 0, dy > 0
    点/捺: dx > 0, dy > 0
    折: 路径包含多个转折
    """
    try:
        # 这里只是演示逻辑，实际需要解析 SVG 命令
        # 很多折笔由多个 L 命令组成
        # 简化处理：根据首尾点位置判断
        commands = path.replace(',', ' ').split()
        points = []
        for i in range(len(commands)):
            if commands[i] in ['M', 'L', 'Q', 'C']:
                try:
                    points.append((float(commands[i+1]), float(commands[i+2])))
                except: pass
        
        if len(points) < 2: return 5 # 默认折
        
        # 如果点数多且有明显的转折，认为是折
        if len(points) > 3:
            # 简单的转折判断：斜率变化大
            return 5
            
        start = points[0]
        end = points[-1]
        dx = end[0] - start[0]
        dy = end[1] - start[1]
        
        if abs(dy) < 100 and dx > 100: return 1 # 横
        if abs(dx) < 100 and dy > 100: return 2 # 竖
        if dx < -50 and dy > 50: return 3 # 撇
        if dx > 50 and dy > 50: return 4 # 捺/点
        return 5 # 折
    except:
        return 5

def download_and_process():
    url = "https://raw.githubusercontent.com/skishore/makemeahanzi/master/graphics.txt"
    print("Downloading graphics data (this may take a while)...")
    response = requests.get(url, stream=True)
    
    char_to_aux = {}
    count = 0
    for line in response.iter_lines():
        if not line: continue
        data = json.loads(line.decode('utf-8'))
        char = data['character']
        paths = data['strokes']
        
        if len(paths) >= 1:
            s1 = get_stroke_type(paths[0])
            s2 = get_stroke_type(paths[1]) if len(paths) >= 2 else 1 # 补位横
            
            idx = (s1 - 1) * 5 + (s2 - 1)
            char_to_aux[char] = chr(ord('a') + idx)
            count += 1
            if count % 1000 == 0: print(f"Processed {count} characters...")
            if count >= 10000: break # 先处理常用字测试
            
    return char_to_aux

if __name__ == "__main__":
    # 实际项目中，我们会把这个结果存入本地缓存或直接更新字典
    res = download_and_process()
    print(f"Total processed: {len(res)}")
    # 示例输出
    for c in "的一是在不":
        print(f"{c}: {res.get(c, 'N/A')}")
