import json
import os

def main():
    data = []
    if not os.path.exists('chars.txt'):
        return

    with open('chars.txt', 'r', encoding='utf-8') as f:
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

    html_content = f"""
<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>单字辅助码对照表</title>
    <link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/bootstrap@5.3.0/dist/css/bootstrap.min.css">
    <link rel="stylesheet" href="https://cdn.datatables.net/1.13.6/css/dataTables.bootstrap5.min.css">
    <style>
        body {{ padding: 20px; background-color: #f0f2f5; font-family: 'Segoe UI', sans-serif; }}
        .container-main {{ background: white; padding: 20px; border-radius: 12px; box-shadow: 0 4px 20px rgba(0,0,0,0.08); max-width: 1200px; margin: auto; }}
        h2 {{ text-align: center; margin-bottom: 20px; color: #0d6efd; font-weight: bold; }}
        .aux-highlight {{ color: #d93025; font-weight: bold; background-color: #fce8e6; padding: 2px 6px; border-radius: 4px; }}
        .char-cell {{ font-size: 1.5em; font-weight: bold; color: #333; }}
        /* 同音字背景色 */
        .pinyin-group-0 {{ background-color: #ffffff !important; }}
        .pinyin-group-1 {{ background-color: #f8f9fa !important; }}
        .table-hover tbody tr.pinyin-group-0:hover {{ background-color: #f1f7ff !important; }}
        .table-hover tbody tr.pinyin-group-1:hover {{ background-color: #f1f7ff !important; }}
    </style>
</head>
<body>
    <div class="container-main">
        <h2>单字辅助码对照表</h2>
        <table id="charsTable" class="table table-hover align-middle" style="width:100%">
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
        $(document).ready(function() {{
            $('#charsTable').DataTable({{
                data: tableData,
                columns: [
                    {{ data: 'pinyin', width: '20%' }},
                    {{ data: 'char', className: 'char-cell', width: '15%' }},
                    {{ 
                        data: 'aux', 
                        width: '20%',
                        render: function(data) {{
                            return '<span class="aux-highlight">' + data + '</span>';
                        }}
                    }},
                    {{ data: 'en', width: '45%' }}
                ],
                createdRow: function(row, data, dataIndex) {{
                    $(row).addClass('pinyin-group-' + data.group);
                }},
                pageLength: 25,
                language: {{ url: 'https://cdn.datatables.net/plug-ins/1.13.6/i18n/zh-CN.json' }},
                order: [[0, 'asc']]
            }});
        }});
    </script>
</body>
</html>
"""
    with open('index_chars.html', 'w', encoding='utf-8') as f:
        f.write(html_content)
    print("Updated index_chars.html with homophone background colors.")

if __name__ == "__main__":
    main()