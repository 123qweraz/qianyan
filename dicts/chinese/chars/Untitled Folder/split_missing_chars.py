import os

def split_file(input_file, num_parts, output_dir):
    if not os.path.exists(output_dir):
        os.makedirs(output_dir)
        print(f"Created directory: {output_dir}")

    with open(input_file, 'r', encoding='utf-8') as f:
        chars = [line.strip() for line in f if line.strip()]

    total_chars = len(chars)
    # Calculate chunk size to distribute as evenly as possible
    chunk_size = (total_chars + num_parts - 1) // num_parts
    
    print(f"Total characters: {total_chars}")
    print(f"Splitting into {num_parts} files (approx {chunk_size} chars each)...")

    for i in range(num_parts):
        start = i * chunk_size
        end = min((i + 1) * chunk_size, total_chars)
        
        part_chars = chars[start:end]
        output_file = os.path.join(output_dir, f"missing_chars_part_{i+1}.txt")
        
        with open(output_file, 'w', encoding='utf-8') as f:
            for char in part_chars:
                f.write(char + '\n')
        
        print(f"Saved {len(part_chars)} characters to {output_file}")

if __name__ == "__main__":
    split_file('missing_chars.txt', 10, 'missing_parts')
