import json
import requests
import time
import os
import sys

# --- Configuration ---
INPUT_FILE = 'chars_output.txt'
OUTPUT_FILE = 'chars_output_trans.txt'
CHECKPOINT_FILE = 'translation_checkpoint.json'

MODEL_NAME = 'gemma4:latest'

BATCH_SIZE = 5
TIMEOUT = 300
DELAY_BETWEEN_BATCHES = 1


def get_translations_batch(words_list):
    url = "http://localhost:11434/api/generate"
    words_str = ", ".join(words_list)

    prompt = f"""
You are a professional Chinese-English lexicographer.

Translate each Chinese word/phrase into English.

Rules:
1. Output ONLY translations separated by semicolons (;)
2. No explanations, no extra text
3. Keep the same order as input
4. One translation per input item
5. Within the same batch, try to use DIFFERENT initial letters for translations when possible

Input:
{words_str}

Output:
"""

    payload = {
        "model": MODEL_NAME,
        "prompt": prompt,
        "stream": False,
        "options": {
            "temperature": 0.1
        }
    }

    try:
        response = requests.post(url, json=payload, timeout=TIMEOUT)
        response.raise_for_status()

        result = response.json().get('response', '').strip()

        # safer parsing
        translated = [t.strip() for t in result.split(';') if t.strip()]

        return translated

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
    if not os.path.exists(INPUT_FILE):
        print(f"Error: {INPUT_FILE} not found.")
        return

    with open(INPUT_FILE, 'r', encoding='utf-8') as f:
        all_words = [line.strip() for line in f if line.strip()]

    total_words = len(all_words)
    start_index = load_checkpoint()

    print(f"Total words: {total_words}")
    if start_index > 0:
        print(f"Resuming from: {start_index}")

    mode = 'a' if start_index > 0 else 'w'

    try:
        with open(OUTPUT_FILE, mode, encoding='utf-8') as out_f:

            for i in range(start_index, total_words, BATCH_SIZE):
                batch = all_words[i:i + BATCH_SIZE]

                print(f"Processing {i}/{total_words} ({i / total_words * 100:.2f}%)", end='\r')

                translations = get_translations_batch(batch)

                # strict check (important)
                if translations and len(translations) == len(batch):
                    for idx, word in enumerate(batch):
                        out_f.write(f"{word}: {translations[idx]}\n")
                else:
                    print(f"\nBatch failed or mismatch at {i}. Retrying individually...")

                    # fallback per item
                    for word in batch:
                        res = get_translations_batch([word])
                        trans = res[0] if (res and len(res) > 0) else "translation_error"
                        out_f.write(f"{word}: {trans}\n")

                out_f.flush()
                save_checkpoint(i + len(batch))

                time.sleep(DELAY_BETWEEN_BATCHES)

    except KeyboardInterrupt:
        print("\nInterrupted. Progress saved.")
        sys.exit(0)

    print(f"\nDone. Total: {total_words}")

    if os.path.exists(CHECKPOINT_FILE):
        os.remove(CHECKPOINT_FILE)


if __name__ == "__main__":
    main()
