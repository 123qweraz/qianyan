import json
import requests
import time
import os
import sys

# --- Configuration ---
INPUT_FILE = 'low_freq.txt'
OUTPUT_FILE = 'low_freq_trans.txt'
CHECKPOINT_FILE = 'translation_checkpoint.json'
MODEL_NAME = 'granite4.1:3b'  # Updated to the exact name found
BATCH_SIZE = 10                # Reduced to 5 for faster individual responses
TIMEOUT = 300                 # Increased to 5 minutes to prevent timeouts
DELAY_BETWEEN_BATCHES = 1    # 1 second delay to let hardware breathe

def get_translations_batch(words_list):
    url = "http://localhost:11434/api/generate"
    words_str = ", ".join(words_list)
    prompt = f"""You are a professional Chinese-English dictionary. 
Translate the following list of Chinese words/phrases into English. 
Provide the only one translations in the exact same order, separated by semicolons. 
Do not include any explanations or extra text, If phrases have the same pronunciation, 
try to use completely different initial letters for translation.

Words: {words_str}

Translations:"""

    payload = {
        "model": MODEL_NAME,
        "prompt": prompt,
        "stream": False,
        "options": {"temperature": 0.3}
    }
    
    try:
        response = requests.post(url, json=payload, timeout=TIMEOUT)
        response.raise_for_status()
        result = response.json().get('response', '').strip()
        # Clean up possible leading/trailing markers the LLM might add
        translated = [t.strip() for t in result.split(';')]
        return translated
    except Exception as e:
        print(f"\nError calling Ollama: {e}")
        return None

def save_checkpoint(index):
    with open(CHECKPOINT_FILE, 'w', encoding='utf-8') as f:
        json.dump({"last_index": index}, f)

def load_checkpoint():
    if os.path.exists(CHECKPOINT_FILE):
        with open(CHECKPOINT_FILE, 'r') as f:
            return json.load(f).get("last_index", 0)
    return 0

def main():
    # 1. Load the words
    if not os.path.exists(INPUT_FILE):
        print(f"Error: {INPUT_FILE} not found.")
        return
        
    with open(INPUT_FILE, 'r', encoding='utf-8') as f:
        all_words = [line.strip() for line in f if line.strip()]

    total_words = len(all_words)
    start_index = load_checkpoint()
    
    print(f"Total words to translate: {total_words}")
    if start_index > 0:
        print(f"Resuming from index: {start_index}")

    # Open output file in append mode
    mode = 'a' if start_index > 0 else 'w'
    
    try:
        with open(OUTPUT_FILE, mode, encoding='utf-8') as out_f:
            for i in range(start_index, total_words, BATCH_SIZE):
                batch = all_words[i : i + BATCH_SIZE]
                
                print(f"Processing: {i}/{total_words} ({(i/total_words)*100:.2f}%)", end='\r')
                
                translations = get_translations_batch(batch)
                
                if translations and len(translations) >= len(batch):
                    for idx, word in enumerate(batch):
                        out_f.write(f"{word}: {translations[idx]}\n")
                else:
                    # Retry individually if batch fails
                    print(f"\nBatch at {i} failed. Retrying individually...")
                    for word in batch:
                        res = get_translations_batch([word])
                        trans = res[0] if (res and len(res) > 0) else "translation_error"
                        out_f.write(f"{word}: {trans}\n")
                
                out_f.flush() # Ensure it's written to disk
                save_checkpoint(i + len(batch))
                
    except KeyboardInterrupt:
        print("\n\nInterrupted by user. Progress saved.")
        sys.exit(0)

    print(f"\n\nSuccess! All {total_words} words translated.")
    if os.path.exists(CHECKPOINT_FILE):
        os.remove(CHECKPOINT_FILE)

if __name__ == "__main__":
    main()
