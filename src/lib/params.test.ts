import { describe, expect, it } from "vitest";
import { fillParams, findParams, literal } from "./params";

describe("query parameters", () => {
  it("finds names, once each, outside strings, comments and casts", () => {
    const sql = "SELECT * FROM orders WHERE customer_id = :customer AND placed_at > :since::date -- not :this\n AND note <> ':nope' AND total > :min AND customer_id <> :customer";
    expect(findParams(sql, "postgres")).toEqual(["customer", "since", "min"]);
  });

  it("leaves casts, assignments, slices, times and dollar quotes alone", () => {
    expect(findParams("SELECT x::text, arr[1:2], '10:30', $$ :body $$ FROM t", "postgres")).toEqual([]);
    expect(findParams("SET @a := 1; SELECT `col:name` FROM t # :c", "mysql")).toEqual([]);
    expect(findParams("SELECT [a:b] FROM t WHERE id = :id", "sqlserver")).toEqual(["id"]);
  });

  it("writes values as safe literals for each database", () => {
    expect(literal("42", "postgres")).toBe("42");
    expect(literal("-1.5", "mysql")).toBe("-1.5");
    // Leading zeros are text, like a postcode.
    expect(literal("007", "postgres")).toBe("'007'");
    expect(literal("O'Brien", "postgres")).toBe("'O''Brien'");
    expect(literal("C:\\temp", "mysql")).toBe("'C:\\\\temp'");
    expect(literal("İstanbul", "sqlserver")).toBe("N'İstanbul'");
    expect(literal(null, "sqlite")).toBe("NULL");
  });

  it("fills every occurrence and nothing else", () => {
    const sql = "SELECT * FROM t WHERE a = :x OR b = :x AND c = ':x' AND d = :y";
    expect(fillParams(sql, { x: "1", y: "it's" }, "postgres")).toBe("SELECT * FROM t WHERE a = 1 OR b = 1 AND c = ':x' AND d = 'it''s'");
    expect(fillParams("SELECT :missing", {}, "postgres")).toBe("SELECT NULL");
  });
});
