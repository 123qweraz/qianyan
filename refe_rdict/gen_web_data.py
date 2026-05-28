import json
import os

def main():
    data = []
    try:
        with open('words_2char.txt', 'r', encoding='utf-8') as f:
            last_pinyin = None
            group_toggle = 0
            for line in f:
                parts = line.strip().split('\t')
                if len(parts) == 4:
                    pinyin = parts[0]
                    if last_pinyin is not None and pinyin != last_pinyin:
                        group_toggle = 1 - group_toggle
                    last_pinyin = pinyin
                    data.append({
                        "pinyin": pinyin,
                        "char": parts[1],
                        "en": parts[2],
                        "aux": parts[3],
                        "group": group_toggle
                    })
    except FileNotFoundError:
        print("words_2char.txt not found.")
        return

    html_content = f"""
<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>两字词辅助码对照表</title>
    <link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/bootstrap@5.3.0/dist/css/bootstrap.min.css">
    <link rel="stylesheet" href="https://cdn.datatables.net/1.13.6/css/dataTables.bootstrap5.min.css">
    <style>
        body {{ padding: 20px; background-color: #f0f2f5; font-family: 'Segoe UI', sans-serif; }}
        .container-main {{ background: white; padding: 20px; border-radius: 12px; box-shadow: 0 4px 20px rgba(0,0,0,0.08); max-width: 1200px; margin: auto; }}
        h2 {{ text-align: center; margin-bottom: 20px; color: #1a73e8; font-weight: bold; }}
        .aux-highlight {{ color: #d93025; font-weight: bold; background-color: #fce8e6; padding: 2px 6px; border-radius: 4px; }}
        .char-cell {{ font-size: 1.2em; font-weight: 500; }}
        /* 同音字背景色 */
        .pinyin-group-0 {{ background-color: #ffffff !important; }}
        .pinyin-group-1 {{ background-color: #f8f9fa !important; }}
        .table-hover tbody tr.pinyin-group-0:hover {{ background-color: #e8f0fe !important; }}
        .table-hover tbody tr.pinyin-group-1:hover {{ background-color: #e8f0fe !important; }}
        #scroll-status {{ position: fixed; top: 10px; right: 20px; background: rgba(0,0,0,0.7); color: white; padding: 5px 15px; border-radius: 20px; font-size: 0.8em; z-index: 1000; }}
    </style>
</head>
<body>
    <div id="scroll-status">自动翻页中... (点击表格停止/恢复)</div>
    <div class="container-main">
        <h2>两字词辅助码对照表</h2>
        <table id="wordsTable" class="table table-hover align-middle" style="width:100%">
            <thead>
                <tr>
                    <th>拼音</th>
                    <th>汉字</th>
                    <th>辅助码</th>
                    <th>英语含义</th>
                </tr>
            </thead>
            <tbody></tbody>
        </table>
    </div>

    <script src="https://code.jquery.com/jquery-3.7.0.min.js"></script>
    <script src="https://cdn.datatables.net/1.13.6/js/jquery.dataTables.min.js"></script>
    <script src="https://cdn.datatables.net/1.13.6/js/dataTables.bootstrap5.min.js"></script>
    <script>
        const tableData = {json.dumps(data, ensure_ascii=False)};
        let autoScroll = true;
        
        $(document).ready(function() {{
            const table = $('#wordsTable').DataTable({{
                data: tableData,
                columns: [
                    {{ data: 'pinyin', width: '20%' }},
                    {{ data: 'char', className: 'char-cell', width: '20%' }},
                    {{ 
                        data: 'aux', 
                        width: '20%',
                        render: function(data) {{
                            return '<span class="aux-highlight">' + data + '</span>';
                        }}
                    }},
                    {{ data: 'en', width: '40%' }}
                ],
                createdRow: function(row, data, dataIndex) {{
                    $(row).addClass('pinyin-group-' + data.group);
                }},
                pageLength: 15,
                language: {{ url: 'https://cdn.datatables.net/plug-ins/1.13.6/i18n/zh-CN.json' }},
                order: [[0, 'asc']]
            }});

            setInterval(function() {{
                if (autoScroll) {{
                    const info = table.page.info();
                    const nextPage = info.page + 1;
                    table.page(nextPage < info.pages ? nextPage : 0).draw('page');
                }}
            }}, 3000);

            $('.container-main').on('click', function() {{
                autoScroll = !autoScroll;
                $('#scroll-status').text(autoScroll ? '自动翻页中... (点击停止)' : '已暂停自动翻页 (点击恢复)');
                $('#scroll-status').css('background', autoScroll ? 'rgba(0,0,0,0.7)' : 'rgba(217,48,37,0.8)');
            }});
        }});
    </script>
</body>
</html>
"""
    with open('index.html', 'w', encoding='utf-8') as f:
        f.write(html_content)
    print("Updated index.html with homophone background colors.")

if __name__ == "__main__":
    main()
