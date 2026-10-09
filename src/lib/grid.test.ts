import { describe, expect, it } from "vitest";
import { describeSummary, summarize } from "./aggregate";
import { formatRows } from "./copyAs";
import type { ColumnMeta } from "./types";

describe("summarize", () => {
  it("adds up numbers and skips NULLs", () => {
    const s = summarize(["10", "2.5", null, "-0.5"]);
    expect(s).toEqual({ count: 4, filled: 3, numbers: { sum: 12, avg: 4, min: -0.5, max: 10 } });
    expect(describeSummary(s)).toBe("Count 3 · Sum 12 · Average 4 · Min -0.5 · Max 10");
  });

  it("only counts when anything isn't a number", () => {
    expect(summarize(["1", "two"]).numbers).toBeNull();
    expect(describeSummary(summarize(["a", "b"]))).toBe("Count 2");
    expect(describeSummary(summarize(["a"]))).toBe("");
  });
});

describe("formatRows", () => {
  const cols: ColumnMeta[] = [
    { name: "id", typeName: "int8", kind: "number" },
    { name: "name", typeName: "text", kind: "text" },
    { name: "active", typeName: "bool", kind: "bool" },
  ];
  const rows = [
    ["1", "O'Brien, \"Bob\"", "true"],
    ["2", null, "false"],
  ];

  it("copies for spreadsheets, CSV and Markdown", () => {
    expect(formatRows("tsv", cols, rows)).toBe("id\tname\tactive\n1\tO'Brien, \"Bob\"\ttrue\n2\t\tfalse");
    expect(formatRows("csv", cols, rows)).toBe('id,name,active\n1,"O\'Brien, ""Bob""",true\n2,,false');
    expect(formatRows("markdown", cols, [["3", "a|b", null]])).toBe("| id | name | active |\n| ---: | --- | --- |\n| 3 | a\\|b | NULL |");
  });

  it("copies JSON with real types", () => {
    expect(JSON.parse(formatRows("json", cols, rows))).toEqual([
      { id: 1, name: "O'Brien, \"Bob\"", active: true },
      { id: 2, name: null, active: false },
    ]);
  });

  it("writes INSERTs quoted for the database", () => {
    expect(formatRows("insert", cols, [rows[0]], { kind: "postgres", table: '"public"."people"' })).toBe(
      'INSERT INTO "public"."people" ("id", "name", "active") VALUES (1, \'O\'\'Brien, "Bob"\', TRUE);',
    );
    expect(formatRows("insert", cols, [["2", "a\\b", "false"]], { kind: "mysql", table: "`people`" })).toBe(
      "INSERT INTO `people` (`id`, `name`, `active`) VALUES (2, 'a\\\\b', 0);",
    );
  });
});

import { defaultLayout, displayColumns, moveColumn, setHidden } from "./columnLayout";

describe("column layout", () => {
  const names = ["id", "name", "email", "city"];
  it("orders, hides and keeps new columns", () => {
    expect(displayColumns(names, defaultLayout())).toEqual([0, 1, 2, 3]);
    const l = { order: ["city", "id"], hidden: ["email"], frozen: 1 };
    expect(displayColumns(names, l)).toEqual([3, 0, 1]);
    expect(displayColumns([...names, "added"], l)).toEqual([3, 0, 1, 4]);
  });
  it("moves by display position, ignoring hidden columns", () => {
    const l = setHidden(defaultLayout(), "name", true);
    const moved = moveColumn(names, l, 2, 0); // city to the front
    expect(displayColumns(names, moved).map((i) => names[i])).toEqual(["city", "id", "email"]);
    expect(displayColumns(names, setHidden(moved, "name", false)).map((i) => names[i])).toEqual(["city", "id", "email", "name"]);
  });
});

describe("SQL Server copy", () => {
  it("brackets names, uses N'' strings and 1/0 booleans", () => {
    const cols: ColumnMeta[] = [
      { name: "full name", typeName: "nvarchar", kind: "text" },
      { name: "active", typeName: "bit", kind: "bool" },
    ];
    expect(formatRows("insert", cols, [["Ayşe", "true"]], { kind: "sqlserver", table: "[dbo].[people]" })).toBe("INSERT INTO [dbo].[people] ([full name], [active]) VALUES (N'Ayşe', 1);");
  });
});
