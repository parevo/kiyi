// Walks through the main screens in WebKit (what Tauri uses on macOS) and saves screenshots.
// Needs `pnpm dev` (Vite on :1420) and `cargo run -p kiyi-devbridge` (:1421) running.
//
//   node dev/shoot.mjs <out-dir> [dark|light]
import { webkit } from "playwright";

const out = process.argv[2] ?? "screenshots";
const theme = process.argv[3] ?? "dark";
const browser = await webkit.launch();
const page = await browser.newPage({ viewport: { width: 1440, height: 900 }, colorScheme: theme });
const errors = [];
page.on("console", (m) => (m.type() === "error" || m.type() === "warning") && errors.push(m.text()));
page.on("pageerror", (e) => errors.push(String(e)));

let n = 0;
const shot = async (name) => {
  await page.waitForTimeout(450);
  const file = `${out}/${String(++n).padStart(2, "0")}-${name}.png`;
  await page.screenshot({ path: file });
  console.log(file);
};
const step = async (name, fn) => {
  try {
    await fn();
  } catch (e) {
    console.log(`step "${name}" failed: ${e.message.split("\n")[0]}`);
  }
};
const button = (name) => page.getByRole("button", { name, exact: false }).first();

// Start from a clean slate: no saved connections in the bridge.
const call = (cmd, body = {}) =>
  fetch(`http://127.0.0.1:1421/invoke/${cmd}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) }).then((r) => r.json());
for (const c of await call("list_connections")) await call("delete_connection", { id: c.id });

await page.goto("http://localhost:1420");
await page.evaluate(() => localStorage.clear());
await page.reload();
await page.waitForTimeout(800);
await shot("welcome");

await step("connect", async () => {
  await page.getByPlaceholder("Paste a connection URL").fill("postgres://kiyi:kiyi@localhost:55432/shop");
  await page.keyboard.press("Enter");
  await page.waitForTimeout(600);
  await shot("connection-dialog");
  await button("Save & connect").click();
  await page.waitForTimeout(1500);
});
await shot("overview");

await step("open orders", async () => {
  await page.locator("main").getByRole("button", { name: /^orders/ }).first().click();
  await page.waitForTimeout(1500);
});
await shot("table-data");

await step("select row", async () => {
  await page.mouse.click(640, 230);
  await page.waitForTimeout(500);
});
await shot("row-details");

await step("filter popover", async () => {
  await button("Filter").click();
  await page.waitForTimeout(400);
});
await shot("filter-popover");

await step("apply filter", async () => {
  await page.getByLabel("Column").first().selectOption("status");
  await page.getByLabel("Value").first().selectOption("paid");
  await button("Apply").click();
  await page.waitForTimeout(1000);
});
await shot("filtered");

await step("search", async () => {
  await button("Clear all").click();
  await page.getByLabel("Search rows or ask AI").fill("shipped");
  await page.keyboard.press("Enter");
  await page.waitForTimeout(1000);
});
await shot("search");

await step("insert row", async () => {
  await button("Clear all").click();
  await button("Insert row").click();
  await page.waitForTimeout(600);
});
await shot("insert-row");
await page.keyboard.press("Escape");

await step("structure", async () => {
  await page.getByRole("button", { name: "Structure", exact: true }).click();
  await page.waitForTimeout(900);
});
await shot("structure");

await step("edit column", async () => {
  await page.getByRole("row").filter({ hasText: "status" }).first().click();
  await page.waitForTimeout(600);
});
await shot("edit-column");
await page.keyboard.press("Escape");

await step("relationships", async () => {
  await page.getByRole("tab", { name: /Relationships/ }).click();
  await page.waitForTimeout(400);
});
await shot("relationships");

await step("new table", async () => {
  await page.getByRole("button", { name: "New table" }).first().click();
  await page.waitForTimeout(800);
});
await shot("new-table");

await step("settings", async () => {
  await page.getByRole("button", { name: "Settings" }).click();
  await page.waitForTimeout(500);
});
await shot("settings");

const real = [...new Set(errors)].filter((e) => !/access control checks/.test(e));
console.log(real.length ? `console errors:\n${real.join("\n")}` : "no console errors");
await browser.close();
