# Third-party notices

The project's original code and contributions by BogdanNL are covered by the
MIT License in LICENSE. The following third-party terms also apply to the
portions derived from blast.

## blast 1.3 — Mark Adler

`src/explode.rs` is an altered Rust implementation based on
[blast 1.3](https://github.com/madler/zlib/tree/master/contrib/blast), including
its canonical Huffman tables and decoding algorithm. It is not the original
blast distribution. Rust adaptation: Copyright (c) 2026
[BogdanNL](https://github.com/BogdanNL).

The original zlib license notice is preserved below and in `src/explode.rs`:

```text
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
```

## STIX — Veit Kannegieser

The archive format was researched using STIX by Veit Kannegieser, including
its Pascal sources and Linux adaptation. The archive parser is a new Rust
implementation.

- [Original sources: ZIP mirror](https://ecsoft2.org/system/files/repository/stix_src.zip)
- [Original sources: author's ARJ archive](https://kannegieser.net/veit/quelle/stix_src.arj)
- [Linux port by Declan Hoare](https://github.com/DeclanHoare/stix)
