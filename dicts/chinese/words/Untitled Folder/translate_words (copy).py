import json
import requests
import time
import os
import sys

# --- Configuration ---
INPUT_FILE = 'low_freq.txt'
OUTPUT_FILE = 'low_freq_trans.txt'
CHECKPOINT_FILE = 'translation_checkpoint.json'

# Ollama model name（按你实际 pull 的来）
MODEL_NAME = "hf.co/acceldium/nllb-200-distilled-600M-GGUF"

BATCH_SIZE = 10
TIMEOUT = 300
DELAY_BETWEEN_BATCHES = 0.5

OLLAMA_URL = "http://localhost:11434/api/generate"


def get_translations_batch(words_list):
    """
    Send a batch to Ollama (NLLB GGUF)
    """
    # 用 || 分隔，减少模型歧义
    prompt = " || ".join(words_list)

    payload = {
        "model": MODEL_NAME,
        "prompt": prompt,
        "stream": False,
        "options": {
            "temperature": 0
        }
    }

    try:
        response = requests.post(OLLAMA_URL, json=payload, timeout=TIMEOUT)
        response.raise_for_status()

        result = response.json().get('response', '').strip()

        # 更鲁棒的解析方式
        translations = [
            t.strip()
            for t in result.replace('\n', ';').replace(',', ';').split(';')
            if t.strip()
        ]

        return translations

    except Exception as e:
        print(f"\nError calling Ollama: {e}")
        return None


def save_checkpoint(index):
    with open(CHECKPOINT_FILE, 'w', encoding='utf-8') as f:
        json.dump({"last_index": index}, f)


def load_checkpoint():
    if os.path.exists(CHECKPOINT_FILE):
        with open(CHECKPOINT_FILE, 'r', encoding='utf-8') as f:
            return json.load(f).get("last_index", 0)
    return 0


def main():
    # 读取输入
    if not os.path.exists(INPUT_FILE):
        print(f"Error: {INPUT_FILE} not found.")
        return

    with open(INPUT_FILE, 'r', encoding='utf-8') as f:
        all_words = [line.strip() for line in f if line.strip()]

    total = len(all_words)
    start = load_checkpoint()

    print(f"Total words: {total}")
    if start > 0:
        print(f"Resume from: {start}")

    mode = 'a' if start > 0 else 'w'

    try:
        with open(OUTPUT_FILE, mode, encoding='utf-8') as out_f:

            for i in range(start, total, BATCH_SIZE):
                batch = all_words[i:i + BATCH_SIZE]

                print(f"Processing {i}/{total} ({i/total*100:.2f}%)", end='\r')

                translations = get_translations_batch(batch)

                # 成功情况
                if translations and len(translations) >= len(batch):
                    for w, t in zip(batch, translations):
                        out_f.write(f"{w}: {t}\n")
                else:
                    # fallback：逐个翻译
                    print(f"\nBatch failed at {i}, retrying one by one...")
                    for w in batch:
                        res = get_translations_batch([w])
                        t = res[0] if res else "translation_error"
                        out_f.write(f"{w}: {t}\n")

                out_f.flush()
                save_checkpoint(i + len(batch))

                time.sleep(DELAY_BETWEEN_BATCHES)

    except KeyboardInterrupt:
        print("\nInterrupted. Progress saved.")
        sys.exit(0)

    print(f"\nDone! Total: {total}")

    if os.path.exists(CHECKPOINT_FILE):
        os.remove(CHECKPOINT_FILE)


if __name__ == "__main__":
    main()
