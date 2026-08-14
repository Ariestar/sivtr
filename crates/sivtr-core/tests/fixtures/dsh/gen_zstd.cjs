// Regenerates tests/fixtures/dsh/session.jsonl.zstd from the FIXTURE constant
// in crates/sivtr-core/src/agents/dsh.rs. The committed `.jsonl.zstd` fixture
// mirrors dsh's on-disk layout — one zstd frame per flush batch (header frame
// first) — so the Rust decoder is exercised against the real encoding.
//
// Run `node crates/sivtr-core/tests/fixtures/dsh/gen_zstd.cjs` after editing
// FIXTURE (the zstd decode test asserts byte-identical round-trips).
const fs = require('node:fs');
const { zstdCompressSync } = require('node:zlib');

// Extract the FIXTURE raw string from dsh.rs so the compressed fixture is
// byte-identical to the Rust test constant.
const src = fs.readFileSync('crates/sivtr-core/src/agents/dsh.rs', 'utf8');
const marker = 'const FIXTURE: &str = r#"';
const start = src.indexOf(marker) + marker.length;
const end = src.indexOf('"#', start);
if (start === -1 || end === -1) throw new Error('FIXTURE not found');
const text = src.slice(start, end);

// Verify the extracted text parses line by line.
const lines = text.split('\n');
let bad = 0;
lines.forEach((line, i) => {
  if (line.trim() !== '') {
    try {
      JSON.parse(line);
    } catch (e) {
      bad += 1;
      console.log('BAD line', i, e.message);
    }
  }
});
console.log('extracted bytes:', Buffer.byteLength(text), 'bad lines:', bad);

// dsh layout: one frame with the header line, one frame with the events.
const parts = text.split('\n');
const headerFrame = zstdCompressSync(Buffer.from(parts[0] + '\n'));
const restFrame = zstdCompressSync(Buffer.from(parts.slice(1, -1).join('\n') + '\n'));
fs.writeFileSync(
  'crates/sivtr-core/tests/fixtures/dsh/session.jsonl.zstd',
  Buffer.concat([headerFrame, restFrame]),
);
console.log('zstd fixture regenerated');
