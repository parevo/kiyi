// End-to-end flows against the dev database through the real UI (WebKit), with screenshots.
// Changes it makes are reverted at the end. Needs Vite (:1420) and kiyi-devbridge (:1421).
//
//   node dev/flows.mjs <out-dir>
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
await page.goto("http://localhost:1420");
await page.evaluate(() => localStorage.clear());
await page.reload();
await page.getByLabel("Connection URL").fill("postgres://kiyi:kiyi@localhost:55432/shop");
await page.keyboard.press("Enter");
await btn("Save & connect").click();
// Leftovers from an interrupted earlier run.
await page.waitForTimeout(800);
await call("execute_script", { id: (await call("list_connections"))[0].id, statements: ["DROP TABLE IF EXISTS kiyi_demo"], kind: "schema" });
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

const real = [...new Set(errors)].filter((e) => !/access control checks/.test(e));
console.log(real.length ? `console errors:\n${real.join("\n")}` : "no console errors");
console.log(failures ? `${failures} failed` : "all flows passed");
await browser.close();
process.exit(failures ? 1 : 0);
