import json
import os
import urllib.request
import re

def download_frequency_data():
    url = "https://raw.githubusercontent.com/hermitdave/FrequencyWords/master/content/2018/en/en_50k.txt"
    print(f"Downloading frequency data from {url}...")
    try:
        with urllib.request.urlopen(url) as response:
            data = response.read().decode('utf-8')
        freq_dict = {}
        for line in data.splitlines():
            parts = line.split()
            if len(parts) == 2:
                word, count = parts[0], int(parts[1])
                freq_dict[word.lower()] = count
        return freq_dict
    except Exception as e:
        print(f"Error downloading frequency data: {e}")
        return {}

def merge_dicts(dir_path, freq_dict):
    files = [
        "combined_dict.json",
        "dict_5_10s.json",
        "dict_enlt5s.json",
        "linux_commands.json"
    ]
    
    merged_data = {}
    
    for file_name in files:
        file_path = os.path.join(dir_path, file_name)
        if not os.path.exists(file_path):
            print(f"Warning: {file_path} not found.")
            continue
            
        print(f"Processing {file_name}...")
        with open(file_path, 'r', encoding='utf-8') as f:
            try:
                data = json.load(f)
                for word, translations in data.items():
                    # Normalize word
                    clean_word = word.strip()
                    if not clean_word:
                        continue
                        
                    # Merge translations
                    trans_str = "; ".join(translations) if isinstance(translations, list) else str(translations)
                    
                    if clean_word not in merged_data:
                        merged_data[clean_word] = {
                            "char": clean_word,
                            "en": trans_str,
                            "trad": clean_word,
                            "weight": freq_dict.get(clean_word.lower(), 0)
                        }
                    else:
                        # Append new translations if not already present
                        existing_trans = merged_data[clean_word]["en"].split("; ")
                        new_trans = [t for t in (translations if isinstance(translations, list) else [translations]) if t not in existing_trans]
                        if new_trans:
                            merged_data[clean_word]["en"] += "; " + "; ".join(new_trans)
            except Exception as e:
                print(f"Error processing {file_name}: {e}")
                
    # Reformat to the requested structure: { "word": [ { ... } ] }
    final_output = {}
    for word, info in merged_data.items():
        final_output[word] = [info]
        
    return final_output

if __name__ == "__main__":
    dir_path = "/home/xiao/Documents/shian/dicts/english/"
    freq_data = download_frequency_data()
    result = merge_dicts(dir_path, freq_data)
    
    output_file = os.path.join(dir_path, "en_dict_final.json")
    with open(output_file, 'w', encoding='utf-8') as f:
        json.dump(result, f, ensure_ascii=False, indent=2)
    
    print(f"Successfully created {output_file} with {len(result)} entries.")
