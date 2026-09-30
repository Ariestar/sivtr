// Regenerates the legacy and v4 `.jsonl.zstd` fixtures from the FIXTURE constant
// in agents/dsh/tests.rs and the synthetic session.v4.jsonl. Each fixture
// mirrors dsh's on-disk layout — one zstd frame per flush batch (header frame
// first) — so the Rust decoder is exercised against the real encoding.
//
// Run `node crates/sivtr-core/tests/fixtures/dsh/gen_zstd.cjs` after editing
// either source (the zstd decode test asserts byte-identical round-trips).
const fs = require('node:fs');
const { zstdCompressSync } = require('node:zlib');

// Extract the FIXTURE raw string from tests.rs so the compressed fixture is
// byte-identical to the Rust test constant.
const src = fs.readFileSync('crates/sivtr-core/src/agents/dsh/tests.rs', 'utf8');
const marker = 'const FIXTURE: &str = r#"';
const markerIndex = src.indexOf(marker);
const start = markerIndex + marker.length;
const end = src.indexOf('"#', start);
if (markerIndex === -1 || end === -1) throw new Error('FIXTURE not found');
const text = src.slice(start, end);

const fixtures = [
  ['crates/sivtr-core/tests/fixtures/dsh/session.jsonl', text],
  ['crates/sivtr-core/tests/fixtures/dsh/session.v4.jsonl',
    fs.readFileSync('crates/sivtr-core/tests/fixtures/dsh/session.v4.jsonl', 'utf8')],
];
for (const [path, content] of fixtures) {
  const parts = content.split('\n');
  parts.filter(line => line.trim() !== '').forEach(line => JSON.parse(line));
  // dsh layout: one frame with the header line, one frame with the events.
  const headerFrame = zstdCompressSync(Buffer.from(parts[0] + '\n'));
  const restFrame = zstdCompressSync(Buffer.from(parts.slice(1, -1).join('\n') + '\n'));
  fs.writeFileSync(path + '.zstd', Buffer.concat([headerFrame, restFrame]));
  console.log('zstd fixture regenerated:', path);
}
