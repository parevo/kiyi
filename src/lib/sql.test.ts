import { describe, expect, it } from "vitest";
import { isDestructive } from "./highlight";
import { splitStatements, statementAt } from "./sql";

const texts = (sql: string, mysql = false) => splitStatements(sql, mysql).map((s) => s.text);

describe("splitStatements", () => {
  it("splits on top-level semicolons only", () => {
    expect(texts("SELECT 1; SELECT 'a;b'; SELECT \"x;y\" FROM t -- c;d\n; /* e;f */ SELECT 2")).toEqual([
      "SELECT 1",
      "SELECT 'a;b'",
      'SELECT "x;y" FROM t -- c;d',
      "/* e;f */ SELECT 2",
    ]);
  });

  it("keeps dollar-quoted function bodies whole", () => {
    expect(texts("CREATE FUNCTION f() RETURNS int AS $body$ BEGIN; RETURN 1; END $body$ LANGUAGE plpgsql; SELECT f()")).toHaveLength(2);
  });

  it("treats a Postgres backslash as a plain character", () => {
    // In standard SQL strings `\` doesn't escape the quote, so this is two statements.
    expect(texts(String.raw`SELECT 'C:\'; DELETE FROM t`)).toEqual([String.raw`SELECT 'C:\'`, "DELETE FROM t"]);
    // E'' strings do use backslash escapes.
    expect(texts(String.raw`SELECT E'it\'s; fine'; SELECT 2`)).toEqual([String.raw`SELECT E'it\'s; fine'`, "SELECT 2"]);
  });

  it("follows MySQL's backslash escapes and # comments", () => {
    expect(texts(String.raw`SELECT 'it\'s; one'; # x;y` + "\nSELECT 2", true)).toEqual([String.raw`SELECT 'it\'s; one'`, "# x;y\nSELECT 2"]);
  });

  it("keeps BEGIN … END blocks whole", () => {
    // SQL Server function, MySQL procedure with nested blocks, SQLite trigger.
    expect(texts("CREATE FUNCTION f(@a int) RETURNS int AS BEGIN DECLARE @b int; SET @b = @a; RETURN @b; END; SELECT dbo.f(1)")).toHaveLength(2);
    expect(
      texts("CREATE PROCEDURE p() BEGIN DECLARE i INT; IF i > 0 THEN SET i = 1; END IF; WHILE i < 3 DO SET i = i + 1; END WHILE; END; CALL p()", true),
    ).toEqual(["CREATE PROCEDURE p() BEGIN DECLARE i INT; IF i > 0 THEN SET i = 1; END IF; WHILE i < 3 DO SET i = i + 1; END WHILE; END", "CALL p()"]);
    expect(texts("CREATE TRIGGER t AFTER UPDATE ON x BEGIN UPDATE x SET a = 1; UPDATE x SET b = 2; END; SELECT 1")).toHaveLength(2);
    expect(texts("BEGIN TRY SELECT 1; END TRY BEGIN CATCH SELECT 2; END CATCH; SELECT 3")).toHaveLength(2);
  });

  it("still splits transactions and CASE expressions", () => {
    expect(texts("BEGIN; UPDATE t SET a = 1; COMMIT;")).toEqual(["BEGIN", "UPDATE t SET a = 1", "COMMIT"]);
    expect(texts("BEGIN TRANSACTION; DELETE FROM t; COMMIT")).toHaveLength(3);
    expect(texts("SELECT CASE WHEN a THEN 1 ELSE 2 END AS x FROM t; SELECT 2")).toHaveLength(2);
    expect(texts("SELECT backend, ends, cased FROM t; SELECT 2")).toHaveLength(2);
  });

  it("finds the statement under the cursor", () => {
    const sql = "SELECT 1;\n\nSELECT 2;";
    expect(statementAt(sql, 3)?.text).toBe("SELECT 1");
    expect(statementAt(sql, sql.length)?.text).toBe("SELECT 2");
  });
});

describe("isDestructive", () => {
  it("flags statements that remove data", () => {
    expect(isDestructive("DROP TABLE t")).toBe(true);
    expect(isDestructive("  delete from t where id = 1")).toBe(true);
    expect(isDestructive("ALTER TABLE t DROP COLUMN a")).toBe(true);
    expect(isDestructive("ALTER TABLE t ADD COLUMN a int")).toBe(false);
    expect(isDestructive("SELECT * FROM deleted_items")).toBe(false);
  });
});
