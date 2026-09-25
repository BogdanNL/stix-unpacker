// Copyright (c) 2026 BogdanNL (https://github.com/BogdanNL), Rust adaptation
// SPDX-License-Identifier: MIT AND Zlib

//! Streaming PKWARE DCL decompression with a 4096-byte history window.
//!
//! This is an altered Rust implementation based on Mark Adler's blast 1.3.
//! See README.md for third-party attribution. The original license follows.

/*
Copyright (C) 2003, 2012, 2013 Mark Adler
version 1.3, 24 Aug 2013

This software is provided 'as-is', without any express or implied
warranty.  In no event will the author be held liable for any damages
arising from the use of this software.

Permission is granted to anyone to use this software for any purpose,
including commercial applications, and to alter it and redistribute it
freely, subject to the following restrictions:

1. The origin of this software must not be misrepresented; you must not
   claim that you wrote the original software. If you use this software
   in a product, an acknowledgment in the product documentation would be
   appreciated but is not required.
2. Altered source versions must be plainly marked as such, and must not be
   misrepresented as being the original software.
3. This notice may not be removed or altered from any source distribution.

Mark Adler    madler@alumni.caltech.edu
*/

use crate::invalid;
use std::io::{self, Read, Write};

const LITERAL_LENGTHS: &[u8] = &[
    11, 124, 8, 7, 28, 7, 188, 13, 76, 4, 10, 8, 12, 10, 12, 10, 8, 23, 8, 9, 7, 6, 7, 8, 7, 6, 55,
    8, 23, 24, 12, 11, 7, 9, 11, 12, 6, 7, 22, 5, 7, 24, 6, 11, 9, 6, 7, 22, 7, 11, 38, 7, 9, 8,
    25, 11, 8, 11, 9, 12, 8, 12, 5, 38, 5, 38, 5, 11, 7, 5, 6, 21, 6, 10, 53, 8, 7, 24, 10, 27, 44,
    253, 253, 253, 252, 252, 252, 13, 12, 45, 12, 45, 12, 61, 12, 45, 44, 173,
];
const LENGTH_LENGTHS: &[u8] = &[2, 35, 36, 53, 38, 23];
const DISTANCE_LENGTHS: &[u8] = &[2, 20, 53, 230, 247, 151, 248];
const BASE: [usize; 16] = [3, 2, 4, 5, 6, 7, 8, 9, 10, 12, 16, 24, 40, 72, 136, 264];
const EXTRA: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8];

struct Bits<R> {
    input: R,
    buffer: usize,
    available: u8,
    consumed: u64,
}

impl<R: Read> Bits<R> {
    fn get(&mut self, count: u8) -> io::Result<usize> {
        while self.available < count {
            let mut byte = [0];
            self.input.read_exact(&mut byte)?;
            self.buffer |= (byte[0] as usize) << self.available;
            self.available += 8;
            self.consumed += 1;
        }
        let value = self.buffer & ((1 << count) - 1);
        self.buffer >>= count;
        self.available -= count;
        Ok(value)
    }
}

struct Huffman {
    counts: [usize; 14],
    symbols: Vec<usize>,
}

impl Huffman {
    fn new(repeats: &[u8]) -> Self {
        let lengths: Vec<_> = repeats
            .iter()
            .flat_map(|&r| std::iter::repeat(r & 15).take((r >> 4) as usize + 1))
            .collect();
        let mut counts = [0; 14];
        for &length in &lengths {
            counts[length as usize] += 1;
        }
        let mut symbols: Vec<_> = (0..lengths.len()).collect();
        symbols.sort_by_key(|&symbol| lengths[symbol]);
        Self { counts, symbols }
    }

    fn decode<R: Read>(&self, bits: &mut Bits<R>) -> io::Result<usize> {
        let (mut code, mut first, mut index) = (0, 0, 0);
        for &count in &self.counts[1..] {
            code |= bits.get(1)? ^ 1;
            if code < first + count {
                return Ok(self.symbols[index + code - first]);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err(invalid("Invalid DCL Huffman code"))
    }
}

/// Decode exactly one stream, enforcing the advertised output size.
/// Returns the number of compressed bytes consumed, including the end marker.
pub fn explode<R: Read, W: Write>(input: R, mut output: W, expected: u64) -> io::Result<u64> {
    let mut bits = Bits {
        input,
        buffer: 0,
        available: 0,
        consumed: 0,
    };
    let coded = bits.get(8)?;
    if coded > 1 {
        return Err(invalid("Invalid DCL literal mode"));
    }
    let dictionary = bits.get(8)? as u8;
    if !(4..=6).contains(&dictionary) {
        return Err(invalid("Invalid DCL dictionary size"));
    }
    let literals = Huffman::new(LITERAL_LENGTHS);
    let lengths = Huffman::new(LENGTH_LENGTHS);
    let distances = Huffman::new(DISTANCE_LENGTHS);
    let mut window = [0u8; 4096];
    let mut next = 0;
    let mut written = 0u64;
    loop {
        let (length, distance, literal) = if bits.get(1)? == 0 {
            let literal = if coded == 1 {
                literals.decode(&mut bits)?
            } else {
                bits.get(8)?
            };
            (1, 0, literal as u8)
        } else {
            let symbol = lengths.decode(&mut bits)?;
            let length = BASE[symbol] + bits.get(EXTRA[symbol])?;
            if length == 519 {
                break;
            }
            let extra = if length == 2 { 2 } else { dictionary };
            let distance = (distances.decode(&mut bits)? << extra) + bits.get(extra)? + 1;
            if distance as u64 > written {
                return Err(invalid("DCL back-reference precedes the start of the file"));
            }
            (length, distance, 0)
        };
        if written + length as u64 > expected {
            return Err(invalid(
                "Decompressed data exceeds the advertised file size",
            ));
        }
        for _ in 0..length {
            window[next] = if distance == 0 {
                literal
            } else {
                window[(next + 4096 - distance) % 4096]
            };
            next += 1;
            if next == window.len() {
                output.write_all(&window)?;
                next = 0;
            }
        }
        written += length as u64;
    }
    if written != expected {
        return Err(invalid(format!(
            "Decompressed size mismatch: expected {expected}, got {written}"
        )));
    }
    output.write_all(&window[..next])?;
    Ok(bits.consumed)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = &[0x00, 0x04, 0x82, 0x24, 0x25, 0x8f, 0x80, 0x7f];

    #[test]
    fn known_dcl_vector_and_overlapping_copy() {
        let mut output = Vec::new();
        assert_eq!(explode(SAMPLE, &mut output, 13).unwrap(), 8);
        assert_eq!(output, b"AIAIAIAIAIAIA");
    }

    #[test]
    fn rejects_truncation_and_wrong_size() {
        for length in 0..SAMPLE.len() {
            assert!(explode(&SAMPLE[..length], io::sink(), 13).is_err());
        }
        for size in [0, 12, 14] {
            assert!(explode(SAMPLE, io::sink(), size).is_err());
        }
        assert!(explode(&[2, 4][..], io::sink(), 0).is_err());
        assert!(explode(&[0, 7][..], io::sink(), 0).is_err());
    }

    #[test]
    fn coded_literals_and_empty_stream() {
        let coded = [
            0x01, 0x04, 0x50, 0x6c, 0xd3, 0xd4, 0xf1, 0x3d, 0xbc, 0xae, 0x99, 0x74, 0x50, 0x06,
            0xfc, 0x03,
        ];
        let mut output = Vec::new();
        explode(&coded[..], &mut output, 13).unwrap();
        assert_eq!(output, b"Hello, world!");
        for dictionary in 4..=6 {
            assert_eq!(
                explode(&[0, dictionary, 1, 255][..], io::sink(), 0).unwrap(),
                4
            );
        }
        assert!(explode(&[0, 4, 0x1f, 0][..], io::sink(), 3).is_err());
    }

    #[test]
    fn back_reference_crosses_multiple_window_flushes() {
        let mut stream = vec![0, 6, 0x82];
        for _ in 0..20 {
            stream.extend_from_slice(&[2, 0xfc, 7]);
        }
        stream.extend_from_slice(&[2, 0xfe, 1]);
        let mut output = Vec::new();
        explode(&stream[..], &mut output, 10361).unwrap();
        assert_eq!(output, vec![b'A'; 10361]);
    }
}
