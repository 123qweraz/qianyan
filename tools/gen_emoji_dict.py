import json
import os
from pypinyin import lazy_pinyin

# 格式: "Emoji": ("英文描述", "中文关键词")
EXACT_DATA = {
    "😄": ("Grinning Face", "笑,微笑,开心,哈哈"),
    "🤣": ("Rolling on the Floor Laughing", "笑哭,给力,给力"),
    "😂": ("Face with Tears of Joy", "笑哭,笑死"),
    "😊": ("Smiling Face with Smiling Eyes", "微笑,幸福"),
    "😍": ("Smiling Face with Heart-Eyes", "色,喜欢,爱心眼"),
    "😘": ("Face Blowing a Kiss", "亲亲,么么哒"),
    "😭": ("Loudly Crying Face", "大哭,流泪,伤心"),
    "😱": ("Face Screaming in Fear", "惊恐,吓死"),
    "😡": ("Pouting Face", "生气,发火,怒"),
    "🤫": ("Shushing Face", "嘘,安静"),
    "🤔": ("Thinking Face", "思考,想一想"),
    "🥳": ("Partying Face", "庆祝,开心"),
    "😎": ("Smiling Face with Sunglasses", "酷,墨镜"),
    "😇": ("Smiling Face with Halo", "天使,善良"),
    "🤡": ("Clown Face", "小丑"),
    "💩": ("Pile of Poo", "大便,便便"),
    "👻": ("Ghost", "鬼,幽灵"),
    "🔥": ("Fire", "火,热门,火热"),
    "✨": ("Sparkles", "闪亮,星星"),
    "⭐": ("Star", "星星,五角星"),
    "💯": ("Hundred Points", "一百分,满分"),
    "👍": ("Thumbs Up", "赞,点赞,好,棒"),
    "👎": ("Thumbs Down", "差,不行,垃圾"),
    "👌": ("OK Hand", "好的,OK"),
    "✌️": ("Victory Hand", "耶,胜利"),
    "🤞": ("Crossed Fingers", "好运"),
    "🫰": ("Hand with Index Finger and Thumb Crossed", "比心,爱心"),
    "🙏": ("Folded Hands", "拜托,祈祷,谢谢"),
    "👏": ("Clapping Hands", "鼓掌,拍手"),
    "💪": ("Flexed Biceps", "加油,力量,强"),
    "🤝": ("Handshake", "握手,合作"),
    "❤️": ("Red Heart", "心,爱心,红心"),
    "💔": ("Broken Heart", "心碎"),
    "🐶": ("Dog Face", "狗,狗狗,汪汪"),
    "🐱": ("Cat Face", "猫,猫咪,喵喵"),
    "🐭": ("Mouse Face", "耗子,老鼠"),
    "🐹": ("Hamster Face", "仓鼠"),
    "🐰": ("Rabbit Face", "兔子,兔"),
    "🦊": ("Fox Face", "狐狸"),
    "🐻": ("Bear Face", "熊"),
    "🐼": ("Panda Face", "熊猫"),
    "🐯": ("Tiger Face", "老虎,虎"),
    "🦁": ("Lion Face", "狮子"),
    "🐮": ("Cow Face", "牛"),
    "🐷": ("Pig Face", "猪"),
    "🐸": ("Frog Face", "青蛙"),
    "🐵": ("Monkey Face", "猴子"),
    "🐔": ("Chicken", "鸡"),
    "🐧": ("Penguin", "企企鹅"),
    "🐦": ("Bird", "鸟"),
    "🐤": ("Baby Chick", "小鸡"),
    "🦄": ("Unicorn Face", "独角兽"),
    "🐝": ("Honeybee", "蜜蜂"),
    "🦋": ("Butterfly", "蝴蝶"),
    "🌹": ("Rose", "玫瑰,花"),
    "🌻": ("Sunflower", "向日葵"),
    "🍎": ("Red Apple", "苹果,红苹果"),
    "🍏": ("Green Apple", "青苹果"),
    "🍌": ("Banana", "香蕉"),
    "🍉": ("Watermelon", "西瓜"),
    "🍓": ("Strawberry", "草莓"),
    "🍒": ("Cherries", "樱桃"),
    " peach": ("Peach", "桃子"),
    "🍍": ("Pineapple", "菠萝"),
    "🍇": ("Grapes", "葡萄"),
    "🍊": ("Tangerine", "橘子"),
    "🍋": ("Lemon", "柠檬"),
    " kiwi": ("Kiwi Fruit", "猕猴桃"),
    "🍅": ("Tomato", "西红柿,番茄"),
    "🌽": ("Ear of Corn", "玉米"),
    "🥩": ("Cut of Meat", "肉,牛排"),
    "🍔": ("Hamburger", "汉堡"),
    "🍟": ("French Fries", "薯条"),
    "🍕": ("Pizza", "披萨"),
    "🍱": ("Bento Box", "便当"),
    "🍜": ("Steaming Bowl", "拉面,面条"),
    "🍲": ("Pot of Food", "火锅"),
    " sushi": ("Sushi", "寿司"),
    " dumpling": ("Dumpling", "饺子"),
    "🍦": ("Soft Serve", "冰淇淋"),
    "🎂": ("Birthday Cake", "蛋糕,生日蛋糕"),
    "🍭": ("Lollipop", "棒棒糖"),
    "🍫": ("Chocolate Bar", "巧克力"),
    "🍺": ("Beer Mug", "啤酒"),
    "🍻": ("Clinking Beer Mugs", "干杯"),
    "🍷": ("Wine Glass", "红酒"),
    "☕": ("Hot Beverage", "咖啡"),
    "🥤": ("Cup with Straw", "可乐,饮料"),
    "💻": ("Laptop Computer", "电脑,笔记本"),
    "📱": ("Mobile Phone", "手机,电话"),
    "☎️": ("Telephone", "电话"),
    "⌚": ("Watch", "手表"),
    "📷": ("Camera", "相机,拍照"),
    "🎮": ("Video Game", "游戏,游戏机"),
    "🚗": ("Automobile", "汽车,开火"),
    "🚲": ("Bicycle", "自行车"),
    "🚀": ("Rocket", "火箭,发射"),
    "✈️": ("Airplane", "飞机"),
    "🏠": ("House", "房子,家"),
    "💰": ("Money Bag", "钱,发财,红包"),
    "💎": ("Gem Stone", "钻石,宝石"),
    "🎁": ("Wrapped Gift", "礼物,送礼"),
    "🎈": ("Balloon", "气球"),
    "✉️": ("Envelope", "邮件,信"),
    "📚": ("Books", "书,读书"),
    "💡": ("Light Bulb", "主意,灯泡"),
    "🔋": ("Battery", "电池"),
    "🔫": ("Pistol", "枪"),
    "💣": ("Bomb", "炸弹"),
    "⚽": ("Soccer Ball", "足球"),
    "🏀": ("Basketball", "篮球"),
    "🌈": ("Rainbow", "彩虹"),
    "☀️": ("Sun", "太阳,晴天"),
    "🌙": ("Crescent Moon", "月亮"),
    "☔": ("Umbrella with Rain Drops", "下雨,雨"),
    "❄️": ("Snowflake", "下雪,雪"),
}

def generate_emoji_dict():
    output_dir = "dicts/chinese/words"
    if not os.path.exists(output_dir):
        os.makedirs(output_dir)
    
    final_dict = {}

    for emoji, (en_desc, zh_keywords) in EXACT_DATA.items():
        kw_list = zh_keywords.split(',')
        for kw in kw_list:
            pys = lazy_pinyin(kw)
            if pys:
                py = "".join(pys)
                if py not in final_dict:
                    final_dict[py] = []
                
                if not any(e["char"] == emoji for e in final_dict[py]):
                    final_dict[py].append({
                        "char": emoji,
                        "trad": emoji,
                        "en": en_desc,
                        "category": "emoji",
                        "weight": 500 # 略低于普通词组
                    })

    output_file = os.path.join(output_dir, "emoji.json")
    with open(output_file, "w", encoding="utf-8") as f:
        json.dump(final_dict, f, ensure_ascii=False, indent=2)
    
    print(f"Emoji dictionary integrated into Chinese words at {output_file}.")

if __name__ == "__main__":
    generate_emoji_dict()
