import { describe, expect, it } from "vitest";
import { buildChart, niceTicks, suggestChart } from "./chartData";
import type { ColumnMeta } from "./types";

const col = (name: string, kind: ColumnMeta["kind"]): ColumnMeta => ({ name, typeName: kind, kind });

describe("suggestChart", () => {
  it("picks a form from the column types", () => {
    expect(suggestChart([col("day", "temporal"), col("sales", "number")], [["2026-01-01", "3"]])?.kind).toBe("number");
    expect(suggestChart([col("day", "temporal"), col("sales", "number")], [["a", "1"], ["b", "2"]])).toEqual({ kind: "line", x: 0, y: [1] });
    expect(suggestChart([col("city", "text"), col("n", "number"), col("sum", "number")], [["a", "1", "2"], ["b", "2", "3"]])).toEqual({ kind: "bar", x: 0, y: [1, 2] });
    expect(suggestChart([col("note", "text")], [["a"], ["b"]])).toEqual({ kind: "bar", x: 0, y: [] });
  });
});

describe("buildChart", () => {
  const cols = [col("city", "text"), col("total", "number")];
  it("adds up repeated categories and sorts biggest first", () => {
    const d = buildChart(cols, [["Ankara", "5"], ["İzmir", "7"], ["Ankara", "4"], [null, "1"]], { kind: "bar", x: 0, y: [1] });
    expect(d.categories).toEqual(["Ankara", "İzmir", "(empty)"]);
    expect(d.series[0].values).toEqual([9, 7, 1]);
  });

  it("counts rows when no measure is picked", () => {
    expect(buildChart(cols, [["a", "1"], ["a", "1"], ["b", "1"]], { kind: "bar", x: 0, y: [] }).series).toEqual([{ name: "Rows", values: [2, 1] }]);
  });

  it("folds a long tail into Other and keeps pies small", () => {
    const rows = Array.from({ length: 10 }, (_, i) => [`c${i}`, String(10 - i)]);
    const pie = buildChart(cols, rows, { kind: "pie", x: 0, y: [1] });
    expect(pie.categories).toHaveLength(6);
    expect(pie.categories[5]).toBe("Other");
    expect(pie.series[0].values[5]).toBe(5 + 4 + 3 + 2 + 1);
    expect(pie.note).toMatch(/Other/);
  });

  it("keeps time in order and falls back from pies that can't work", () => {
    const t = [col("day", "temporal"), col("n", "number"), col("m", "number")];
    const d = buildChart(t, [["2026-02-01", "1", "2"], ["2026-01-01", "3", "4"]], { kind: "pie", x: 0, y: [1, 2] });
    expect(d.kind).toBe("bar");
    expect(d.categories).toEqual(["2026-01-01", "2026-02-01"]);
    expect(d.note).toMatch(/one measure/);
  });
});

describe("needsSeparateCharts", () => {
  it("splits series whose sizes differ too much for one axis", async () => {
    const { needsSeparateCharts } = await import("./chartData");
    expect(needsSeparateCharts([{ name: "orders", values: [3, 9] }, { name: "spent", values: [900, 25_000] }])).toBe(true);
    expect(needsSeparateCharts([{ name: "a", values: [3, 9] }, { name: "b", values: [5, 40] }])).toBe(false);
  });
});

describe("niceTicks", () => {
  it("rounds to clean steps", () => {
    expect(niceTicks(0, 937)).toEqual([0, 200, 400, 600, 800, 1000]);
    expect(niceTicks(-3, 7)).toEqual([-4, -2, 0, 2, 4, 6, 8]);
  });
});
