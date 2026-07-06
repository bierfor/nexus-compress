#!/usr/bin/env python3
"""Generate a synthetic compression benchmark corpus.

We want a mix of:
  - high-entropy random (worst case)
  - medium-entropy English-like text
  - low-entropy repetitive data
  - structured data (JSON, code)
  - binary patterns
"""
import json
import os
import random
import string

CORPUS = os.path.join(os.path.dirname(__file__), "..", "corpus")
os.makedirs(CORPUS, exist_ok=True)


def write(name, data):
    with open(os.path.join(CORPUS, name), "wb") as f:
        f.write(data)
    print(f"  wrote {name}: {len(data)} bytes")


# 1. random (incompressible)
write(
    "random.bin",
    bytes(random.randint(0, 255) for _ in range(256 * 1024)),
)

# 2. repetitive (highly compressible)
write(
    "repetitive.bin",
    b"abcdefgh" * 32 * 1024,
)

# 3. English-like text
words = (
    "the quick brown fox jumps over the lazy dog "
    "pack my box with five dozen liquor jugs "
    "how vexingly quick daft zebras jump "
    "sphinx of black quartz judge my vow "
    "lorem ipsum dolor sit amet consectetur adipiscing elit "
).split()
random.seed(42)
text = " ".join(random.choice(words) for _ in range(32 * 1024))
write("text.txt", text.encode())

# 4. JSON structured data
records = [
    {
        "id": i,
        "name": random.choice(words),
        "email": f"user{i}@example.com",
        "tags": random.sample(words, k=3),
        "score": random.randint(0, 1000),
    }
    for i in range(4000)
]
write("data.json", json.dumps(records, indent=2).encode())

# 5. source code (Rust-like)
src = '''//! Auto-generated synthetic source.
use std::collections::HashMap;

pub fn process(input: &[u8]) -> HashMap<String, usize> {
    let mut counts: HashMap<u8, usize> = HashMap::new();
    for &b in input {
        *counts.entry(b).or_insert(0) += 1;
    }
    let mut out: HashMap<String, usize> = HashMap::new();
    for (k, v) in counts.iter() {
        out.insert(format!("byte_{:02x}", k), *v);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty() {
        assert!(process(b"").is_empty());
    }
}
'''
# repeat the source many times to get a sizeable file
write("code.rs", (src * 200).encode())

# 6. mixed: a file containing patches of all the above (tests classifier)
mixed = b""
mixed += os.urandom(16 * 1024)
mixed += b"hello world hello world hello world\n" * 1024
mixed += json.dumps(records[:100]).encode()
mixed += b"\x00\x01\x02\x03" * 8 * 1024
write("mixed.bin", mixed)

print("corpus ready in", os.path.abspath(CORPUS))