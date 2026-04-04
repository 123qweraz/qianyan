import json
import math

def get_stroke_type(median):
    """
    根据 median 点列识别笔画类型 (1横, 2竖, 3撇, 4捺, 5折)
    数据集坐标: x 增加向右, y 减小向下。
    """
    if not median or len(median) < 2: return 1
    
    start = median[0]
    end = median[-1]
    dx = end[0] - start[0]
    dy = end[1] - start[1]
    
    # 1. 识别“折” (Turning)
    if len(median) > 2:
        max_dist = 0
        A = start[1] - end[1]
        B = end[0] - start[0]
        C = start[0]*end[1] - end[0]*start[1]
        denom = (A*A + B*B)**0.5
        if denom > 10:
            for p in median[1:-1]:
                dist = abs(A*p[0] + B*p[1] + C) / denom
                max_dist = max(max_dist, dist)
            # 调高阈值：撇笔往往有弧度，不应视为折。
            # 真正的折往往有明显的拐弯点
            if max_dist > 150: 
                return 5

    # 2. 基本方向判断
    # 撇: 向左下 dx < 0, dy < 0
    if dx < 0 and dy < 0: return 3
    # 竖: 垂直向下 dy < -100, dx 较小
    if dy < -100 and abs(dx) < abs(dy) * 0.4: return 2
    # 捺/点: 向右下 dx > 0, dy < -50
    if dx > 0 and dy < -50: return 4
    # 横: 向右 dx > 0, dy 较小
    if dx > 0 and abs(dy) < dx * 0.5: return 1
    
    return 5
