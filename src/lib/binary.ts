/** Binary cells arrive as hex text: `0x…` (MySQL, SQLite, SQL Server) or `\x…` (PostgreSQL). */
export function hexToBytes(value: string): Uint8Array | null {
  const hex = value.replace(/^(0x|\\x)/i, "");
  if (hex.length % 2 !== 0 || /[^0-9a-f]/i.test(hex)) return null;
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return out;
}

const SIGNATURES: { type: string; mime?: string; bytes: (number | null)[] }[] = [
  { type: "PNG image", mime: "image/png", bytes: [0x89, 0x50, 0x4e, 0x47] },
  { type: "JPEG image", mime: "image/jpeg", bytes: [0xff, 0xd8, 0xff] },
  { type: "GIF image", mime: "image/gif", bytes: [0x47, 0x49, 0x46, 0x38] },
  { type: "WebP image", mime: "image/webp", bytes: [0x52, 0x49, 0x46, 0x46, null, null, null, null, 0x57, 0x45, 0x42, 0x50] },
  { type: "PDF document", bytes: [0x25, 0x50, 0x44, 0x46] },
  { type: "ZIP archive (also .docx, .xlsx)", bytes: [0x50, 0x4b, 0x03, 0x04] },
  { type: "gzip archive", bytes: [0x1f, 0x8b] },
];

/** What the bytes probably are, from their first few bytes. */
export function sniff(bytes: Uint8Array): { type: string; mime?: string } | null {
  const hit = SIGNATURES.find((s) => s.bytes.length <= bytes.length && s.bytes.every((b, i) => b === null || bytes[i] === b));
  if (hit) return { type: hit.type, mime: hit.mime };
  const sample = bytes.subarray(0, 512);
  if (sample.length > 0 && sample.every((b) => b === 9 || b === 10 || b === 13 || (b >= 32 && b < 127))) return { type: "Plain text" };
  return null;
}

export function formatSize(n: number): string {
  if (n < 1024) return `${n} ${n === 1 ? "byte" : "bytes"}`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

/** Classic hex dump: offset, 16 bytes in hex, and the printable characters. */
export function hexDump(bytes: Uint8Array, limit: number): string {
  const lines: string[] = [];
  const end = Math.min(bytes.length, limit);
  for (let off = 0; off < end; off += 16) {
    const row = bytes.subarray(off, Math.min(off + 16, end));
    const hex = Array.from(row, (b) => b.toString(16).padStart(2, "0")).join(" ");
    const text = Array.from(row, (b) => (b >= 32 && b < 127 ? String.fromCharCode(b) : ".")).join("");
    lines.push(`${off.toString(16).padStart(8, "0")}  ${hex.padEnd(47)}  ${text}`);
  }
  return lines.join("\n");
}
