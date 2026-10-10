import { describe, expect, it } from "vitest";
import { formatSize, hexDump, hexToBytes, sniff } from "./binary";

describe("binary values", () => {
  it("reads both hex spellings", () => {
    expect(Array.from(hexToBytes("0x89504E47")!)).toEqual([0x89, 0x50, 0x4e, 0x47]);
    expect(Array.from(hexToBytes("\\x0aff")!)).toEqual([10, 255]);
    expect(hexToBytes("0xabc")).toBeNull();
    expect(hexToBytes("hello")).toBeNull();
  });

  it("recognises common files", () => {
    expect(sniff(hexToBytes("0x89504e470d0a1a0a")!)?.mime).toBe("image/png");
    expect(sniff(hexToBytes("0x524946460000000057454250")!)?.mime).toBe("image/webp");
    expect(sniff(new TextEncoder().encode("hello\nworld"))?.type).toBe("Plain text");
    expect(sniff(new Uint8Array([0, 1, 2]))).toBeNull();
  });

  it("dumps and sizes", () => {
    expect(hexDump(new TextEncoder().encode("Kiyi!"), 100)).toBe(`00000000  ${"4b 69 79 69 21".padEnd(47)}  Kiyi!`);
    expect(formatSize(1)).toBe("1 byte");
    expect(formatSize(2048)).toBe("2.0 KB");
  });
});
