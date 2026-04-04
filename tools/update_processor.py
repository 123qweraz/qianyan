import re
import os

file_path = 'src/engine/processor.rs'
content = open(file_path, 'r', encoding='utf-8').read()

# 1. Update ParsedPart
content = re.sub(
    r'struct ParsedPart \{.*?pinyin: String,.*?stroke_aux: Option<String>,.*?english_aux: Option<String>,.*?specified_idx: Option<usize>,.*?raw: String,.*?\}',
    'struct ParsedPart {\n    pinyin: String,\n    stroke_aux: Option<String>,\n    english_aux: Option<String>,\n    specified_idx: Option<usize>,\n    raw: String,\n}',
    content, flags=re.DOTALL
)
# Case if it was not already updated in some failed run
if 'stroke_aux: Option<String>' not in content:
    content = re.sub(
        r'struct ParsedPart \{.*?pinyin: String,.*?aux_code: Option<String>,.*?specified_idx: Option<usize>,.*?raw: String,.*?\}',
        'struct ParsedPart {\n    pinyin: String,\n    stroke_aux: Option<String>,\n    english_aux: Option<String>,\n    specified_idx: Option<usize>,\n    raw: String,\n}',
        content, flags=re.DOTALL
    )

# 2. Update parse_buffer
new_parse_buffer = "    fn parse_buffer(&self) -> Vec<ParsedPart> {\n        let buffer_normalized = strip_tones(&self.buffer);\n        let parts: Vec<&str> = buffer_normalized.split(' ').filter(|s| !s.is_empty()).collect();\n        let mut result = Vec::new();\n\n        for part in parts {\n            let mut pinyin = String.new();\n            let mut stroke_aux = None;\n            let mut english_aux = None;\n            let mut specified_idx = None;\n\n            // Find pinyin end: first ';', digit, or uppercase (if not at start)\n            let pinyin_end = part.char_indices().find(|(i, c)| {\n                *c == ';' || c.is_ascii_digit() || (*i > 0 && c.is_ascii_uppercase())\n            }).map(|(i, _)| i).unwrap_or(part.len());\n\n            pinyin = part[..pinyin_end].to_string();\n            let mut rest = &part[pinyin_end..];\n\n            if rest.starts_with(';') {\n                rest = &rest[1..]; // skip ';'\n                let stroke_end = rest.find(|c: char| c.is_ascii_digit() || c.is_ascii_uppercase()).unwrap_or(rest.len());\n                let s = &rest[..stroke_end];\n                if !s.is_empty() { stroke_aux = Some(s.to_string()); }\n                rest = &rest[stroke_end..];\n            }\n\n            if !rest.is_empty() && rest.chars().next().map_or(false, |c| c.is_ascii_uppercase()) {\n                let english_end = rest.find(|c: char| c.is_ascii_digit()).unwrap_or(rest.len());\n                let e = &rest[..english_end];\n                if !e.is_empty() { english_aux = Some(e.to_string()); }\n                rest = &rest[english_end..];\n            }\n\n            if !rest.is_empty() && rest.chars().next().map_or(false, |c| c.is_ascii_digit()) {\n                specified_idx = rest.parse().ok();\n            }\n\n            result.push(ParsedPart {\n                pinyin,\n                stroke_aux,\n                english_aux,\n                specified_idx,\n                raw: part.to_string(),\n            });\n        }\n        result\n    }"

content = re.sub(r'fn parse_buffer\(&self\) -> Vec<ParsedPart> \{.*?^\s*result\s*\}\s*', new_parse_buffer + '\n    ', content, flags=re.DOTALL | re.MULTILINE)

# 3. Update handle_composing to handle Semicolon
if 'key == VirtualKey::Semicolon' not in content:
    content = content.replace(
        'match key {',
        "if key == VirtualKey::Semicolon && !shift_pressed {\n            self.buffer.push(';');\n            if let Some(act) = self.lookup() { return act; }\n            return self.update_phantom_action();\n        }\n\n        match key {"
    )

# 4. Update lookup filtering
new_lookup_filter = "        for m in last_matches_raw {\n            let last_part = raw_parsed.last();\n            \n            // Filter by stroke_aux\n            if let Some(ref aux) = last_part.and_then(|p| p.stroke_aux.as_ref()) {\n                let aux_lower = aux.to_lowercase();\n                if !m.4.to_lowercase().starts_with(&aux_lower) {\n                    continue;\n                }\n            }\n            \n            // Filter by english_aux\n            if let Some(ref aux) = last_part.and_then(|p| p.english_aux.as_ref()) {\n                let aux_lower = aux.to_lowercase();\n                let en_parts: Vec<&str> = m.3.split(',').map(|s| s.trim()).collect();\n                let mut matched = false;\n                for p in en_parts {\n                    if p.to_lowercase().starts_with(&aux_lower) {\n                        matched = true;\n                        break;\n                    }\n                }\n                if !matched {\n                    continue;\n                }\n            }\n\n            if seen.insert(m.0.clone()) { final_matches.push(m); }\n        }"

content = re.sub(r'for m in last_matches_raw \{.*?^\s*if seen\.insert\(m\.0\.clone\(\)\) \{ final_matches\.push\(m\); \}\s*\}', new_lookup_filter, content, flags=re.DOTALL | re.MULTILINE)

# 5. Fixed references to aux_code in other places
content = content.replace('part.aux_code.is_some()', 'part.stroke_aux.is_some() || part.english_aux.is_some()')

with open(file_path, 'w', encoding='utf-8') as f:
    f.write(content)
print("Updated successfully")
