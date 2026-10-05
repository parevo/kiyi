// End-to-end flows against the dev database through the real UI (WebKit), with screenshots.
// Changes it makes are reverted at the end. Needs Vite (:1420) and kiyi-devbridge (:1421).
//
//   node dev/flows.mjs <out-dir>
import { existsSync, readFileSync, unlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { webkit } from "playwright";

const out = process.argv[2] ?? "screenshots";
const browser = await webkit.launch();
const page = await browser.newPage({ viewport: { width: 1440, height: 900 }, colorScheme: "dark" });
const errors = [];
page.on("console", (m) => (m.type() === "error" || m.type() === "warning") && errors.push(m.text()));
page.on("pageerror", (e) => errors.push(String(e)));

let n = 0;
let failures = 0;
const shot = async (name) => {
  await page.waitForTimeout(400);
  const file = `${out}/f${String(++n).padStart(2, "0")}-${name}.png`;
  await page.screenshot({ path: file });
  console.log(file);
};
const check = async (name, fn) => {
  try {
    await fn();
    console.log(`ok   ${name}`);
  } catch (e) {
    failures++;
    console.log(`FAIL ${name}: ${e.message.split("\n")[0]}`);
    await shot(`fail-${name.replace(/\W+/g, "-")}`);
  }
};
const toast = (text) => page.getByRole("status").getByText(text, { exact: false }).first().waitFor({ timeout: 8000 });
const btn = (name, exact = false) => page.getByRole("button", { name, exact }).first();
const call = (cmd, body = {}) =>
  fetch(`http://127.0.0.1:1421/invoke/${cmd}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) }).then((r) => r.json());

for (const c of await call("list_connections")) await call("delete_connection", { id: c.id });
const tmp = (name) => join(tmpdir(), `kiyi-flow-${process.pid}-${name}`);
const setDialogPath = (path) => page.evaluate((p) => (window.__kiyiDialogPath = p), path);
await page.goto("http://localhost:1420");
await page.evaluate(() => localStorage.clear());
await page.reload();
await page.getByLabel("Connection URL").fill("postgres://kiyi:kiyi@localhost:55432/shop");
await page.keyboard.press("Enter");
await btn("Save & connect").click();
// Leftovers from an interrupted earlier run, and a table to import into.
await page.waitForTimeout(800);
const connId = (await call("list_connections"))[0].id;
await call("execute_script", {
  id: connId,
  statements: ["DROP TABLE IF EXISTS kiyi_demo", "DROP TABLE IF EXISTS kiyi_flow_import", "CREATE TABLE kiyi_flow_import (id serial PRIMARY KEY, name text NOT NULL, city text)"],
  kind: "schema",
});
await page.locator("aside").first().getByRole("button", { name: "Reload list" }).click();
await page.waitForTimeout(800);
await page.locator("main").getByRole("button", { name: /^orders/ }).first().click();
await page.waitForTimeout(1500);

await check("edit a value in the row panel saves it", async () => {
  await page.mouse.click(700, 140); // first row
  const total = page.getByRole("complementary", { name: "Row details" }).locator("input").nth(2);
  await total.fill("123.45");
  await total.press("Enter");
  await toast("Saved");
  await shot("saved-toast");
});

await check("undo restores the old value", async () => {
  await page.getByRole("button", { name: "Undo" }).click();
  await page.waitForTimeout(1500);
  const rows = await call("browse_table", { id: (await call("list_connections"))[0].id, request: { schema: "public", table: "orders", filters: [{ column: "id", op: "eq", value: "1" }], rawWhere: null, sort: [], tiebreak: [], limit: 1, offset: 0 } });
  if (rows.rows[0][3] !== "1.99") throw new Error(`total is ${rows.rows[0][3]}`);
});

await check("insert a row from the sheet", async () => {
  await btn("Insert row").click();
  const sheet = page.getByRole("dialog");
  await sheet.locator("input").nth(1).fill("1");
  await sheet.locator("input").nth(1).press("Tab");
  await sheet.locator("input").nth(2).fill("42.00");
  await sheet.locator("input").nth(2).press("Tab");
  await shot("insert-filled");
  await btn("Save row").click();
  await toast("Row added");
});

await check("delete the inserted row asks first", async () => {
  await page.getByLabel("Search rows or ask AI").fill("42.00");
  await page.keyboard.press("Enter");
  await page.waitForTimeout(1200);
  await page.mouse.click(278, 183); // row marker of the first match
  await btn("Delete row").click();
  await shot("delete-confirm");
  await page.getByRole("dialog").getByRole("button", { name: "Delete row" }).click();
  await toast("Row deleted");
  await btn("Clear all").click();
});

await check("add a column", async () => {
  await page.getByRole("button", { name: "Structure", exact: true }).click();
  await btn("Add column").click();
  const sheet = page.getByRole("dialog");
  await sheet.locator("input").first().fill("note");
  await shot("add-column");
  await sheet.getByRole("button", { name: "Add column" }).click();
  await toast("Add column");
  await page.getByRole("row").filter({ hasText: "note" }).first().waitFor();
});

await check("deleting a column shows a plain-language warning", async () => {
  await page.getByRole("row").filter({ hasText: "note" }).first().click();
  await btn("Delete column").click();
  await page.getByText("and all of its data").waitFor();
  await shot("delete-column-review");
  await btn("Apply anyway").click();
  await page.waitForTimeout(1200);
  if (await page.getByRole("row").filter({ hasText: /^note/ }).count()) throw new Error("column still listed");
});

await check("create a table, then delete it", async () => {
  await page.getByRole("button", { name: "New table" }).first().click();
  await page.getByPlaceholder("e.g. customers").fill("kiyi_demo");
  await btn("Create table").click();
  await toast("Create table");
  await page.waitForTimeout(1200);
  await shot("created-table");
  const row = page.getByRole("complementary").getByText("kiyi_demo", { exact: true });
  await page.locator("aside").first().getByText("kiyi_demo", { exact: true }).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Delete table…" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "Delete permanently" }).click();
  await page.waitForTimeout(1000);
  if (await page.locator("aside").first().getByText("kiyi_demo", { exact: true }).count()) throw new Error("table still listed");
  void row;
});

await check("ask AI without a provider opens Settings › AI", async () => {
  await page.locator("aside").first().getByText("orders", { exact: true }).click();
  await page.getByRole("button", { name: "Data", exact: true }).click();
  await page.getByLabel("Search rows or ask AI").fill("paid orders over 100");
  await btn("Ask AI").click();
  await page.getByText("Choose a provider to get started").waitFor();
  await shot("ai-setup");
  await page.keyboard.press("Escape");
});

await check("export the filtered view to CSV", async () => {
  await page.keyboard.press("Escape");
  await page.locator("aside").first().getByText("orders", { exact: true }).click();
  await page.getByLabel("Search rows or ask AI").fill("shipped");
  await page.keyboard.press("Enter");
  await page.waitForTimeout(800);
  const path = tmp("orders.csv");
  await setDialogPath(path);
  await page.getByRole("button", { name: "More", exact: true }).click();
  await page.getByRole("menuitem", { name: "Export as CSV…" }).click();
  await toast("Exported");
  const lines = readFileSync(path, "utf8").trim().split("\n");
  if (lines.length !== 16_667 + 1) throw new Error(`exported ${lines.length - 1} rows`);
  if (!lines.slice(1).every((l) => l.includes("shipped"))) throw new Error("export ignored the search");
  unlinkSync(path);
  await btn("Clear all").click();
});

await check("import a CSV through the sheet", async () => {
  const path = tmp("people.csv");
  writeFileSync(path, "Name,City\nAyşe,İzmir\n\"Lee, Ann\",\nBob,Paris\n");
  await page.locator("aside").first().getByText("kiyi_flow_import", { exact: true }).click();
  await page.waitForTimeout(800);
  await setDialogPath(path);
  await page.getByRole("button", { name: "More", exact: true }).click();
  await page.getByRole("menuitem", { name: "Import from CSV…" }).click();
  await page.getByRole("button", { name: "Import 3 rows" }).waitFor();
  await shot("import-sheet");
  await page.getByRole("button", { name: "Import 3 rows" }).click();
  await toast("Imported 3 rows");
  const r = await call("browse_table", { id: connId, request: { schema: "public", table: "kiyi_flow_import", filters: [], rawWhere: null, sort: [], tiebreak: ["id"], limit: 10, offset: 0 } });
  if (r.rows.length !== 3 || r.rows[1][1] !== "Lee, Ann" || r.rows[1][2] !== null) throw new Error(JSON.stringify(r.rows));
  unlinkSync(path);
});

await check("test a connection through an SSH tunnel", async () => {
  await page.locator("aside").first().getByRole("button", { name: "Choose connection" }).click();
  await btn("New connection").click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Connection URL").fill("postgres://kiyi:kiyi@postgres:5432/shop");
  await page.waitForTimeout(400);
  await dialog.getByRole("radio", { name: "SSH tunnel" }).click();
  await dialog.getByLabel("SSH host").fill("127.0.0.1");
  await dialog.getByLabel("SSH port").fill("52222");
  await dialog.getByLabel("SSH user").fill("kiyi");
  await dialog.getByRole("radio", { name: "Password" }).last().click();
  await dialog.getByLabel("SSH password").fill("kiyi");
  await dialog.getByRole("button", { name: "Test connection" }).click();
  await dialog.getByText("Signed in as kiyi").waitFor({ timeout: 20000 });
  await dialog.getByText(/PostgreSQL 17/).waitFor();
  await shot("ssh-tunnel-test");
  await dialog.getByRole("button", { name: "Close" }).click();
});

await check("open a new SQLite database and create a table", async () => {
  const path = tmp("notes.db");
  await page.locator("aside").first().getByRole("button", { name: "Choose connection" }).click();
  await btn("New connection").click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Database type").selectOption("sqlite");
  await setDialogPath(path);
  await dialog.getByRole("button", { name: "New database…" }).click();
  await page.waitForTimeout(300);
  await shot("sqlite-dialog");
  await dialog.getByRole("button", { name: "Save & connect" }).click();
  await page.getByText("New table").first().waitFor();
  await page.getByRole("button", { name: "New table" }).first().click();
  await page.getByPlaceholder("e.g. customers").fill("todo");
  await btn("Create table").click();
  await toast("Create table");
  await page.waitForTimeout(800);
  await shot("sqlite-table");
  if (!existsSync(path)) throw new Error("database file not created");
});

await page.evaluate(() => 0);
await call("execute_script", { id: connId, statements: ["DROP TABLE IF EXISTS kiyi_flow_import"], kind: "schema" }).catch(() => {});

const real = [...new Set(errors)].filter((e) => !/access control checks/.test(e));
console.log(real.length ? `console errors:\n${real.join("\n")}` : "no console errors");
console.log(failures ? `${failures} failed` : "all flows passed");
await browser.close();
process.exit(failures ? 1 : 0);
