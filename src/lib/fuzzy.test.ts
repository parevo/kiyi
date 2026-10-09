import { describe, expect, it } from "vitest";
import { fuzzyFilter, fuzzyScore } from "./fuzzy";

describe("fuzzy matching", () => {
  it("prefers whole substrings, then word starts", () => {
    const items = ["order_items", "orders", "customer_orders", "settings"];
    // Exact substrings first; "order_items" still matches as a subsequence, after them.
    expect(fuzzyFilter(items, "orders", (x) => x)).toEqual(["orders", "customer_orders", "order_items"]);
    expect(fuzzyFilter(items, "oi", (x) => x)[0]).toBe("order_items");
    expect(fuzzyScore("xyz", "orders")).toBeNull();
  });
  it("keeps the original order without a query", () => {
    expect(fuzzyFilter(["b", "a"], "  ", (x) => x)).toEqual(["b", "a"]);
  });
});
