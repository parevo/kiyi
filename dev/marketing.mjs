// Captures the website's product screenshots from the real UI, in dark and light, at 2x.
// Needs Vite (:1420), kiyi-devbridge (:1421), the dev databases with the `demo` seed
// (dev/seed/demo.sql) and the Move data pair (dev/seed/move-legacy.sql, move-target.sql), and cwebp.
//
//   node dev/marketing.mjs site/assets/shots
//   node dev/marketing.mjs site/assets/shots dashboard results quality   (only these scenes)
//
// The "Ask AI" scene uses a local stand-in for the model (an OpenAI-compatible server
// with a canned answer), so the screenshot doesn't depend on an API key.
import { execFileSync } from "node:child_process";
import { createServer } from "node:http";
import { mkdirSync, unlinkSync } from "node:fs";
import { webkit } from "playwright";

const out = process.argv[2] ?? "site/assets/shots";
/** Scenes of the 0.6 features, which can be captured on their own. */
const LATER = ["dashboard", "results", "quality"];
const only = process.argv.slice(3);
mkdirSync(out, { recursive: true });

const AI_ANSWER = {
  filters: [
    { column: "plan", op: "eq", value: "pro" },
    { column: "country", op: "eq", value: "Germany" },
    { column: "lifetime_value", op: "gt", value: "1000" },
  ],
  sort: [{ column: "lifetime_value", descending: true }],
  condition: "",
  explanation: "Pro customers in Germany who spent over $1,000, biggest spenders first",
};
const ASK_ANSWER = {
  sql: "SELECT date_trunc('month', placed_at)::date AS month_start, sum(total) AS revenue FROM orders WHERE status <> 'refunded' AND placed_at >= date_trunc('month', now()) - interval '12 months' AND placed_at < date_trunc('month', now()) GROUP BY 1 ORDER BY 1",
  explanation: "Revenue per month over the last 12 months, refunds left out",
  chart: { kind: "line", x: "month_start", y: ["revenue"] },
};
const stub = createServer((req, res) => {
  let body = "";
  req.on("data", (c) => (body += c));
  req.on("end", () => {
    res.setHeader("content-type", "application/json");
    if (req.url.endsWith("/models")) return res.end(JSON.stringify({ data: [{ id: "llama3.2" }] }));
    const answer = body.includes("ONE read-only SQL query") ? ASK_ANSWER : AI_ANSWER;
    res.end(JSON.stringify({ choices: [{ message: { content: JSON.stringify(answer) } }] }));
  });
}).listen(8787);

const call = (cmd, body = {}) =>
  fetch(`http://127.0.0.1:1421/invoke/${cmd}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) }).then((r) => r.json());

async function reset() {
  for (const c of await call("list_connections")) await call("delete_connection", { id: c.id });
  for (const p of (await call("ai_settings")).providers) await call("delete_ai_provider", { id: p.id });
}

async function capture(theme) {
  await reset();
  const ollama = await call("save_ai_provider", {
    provider: { id: "", name: "Ollama", kind: "openAi", baseUrl: "http://127.0.0.1:8787/v1", model: "llama3.2", preset: "ollama" },
    key: null,
  });
  await call("save_ai_provider", {
    provider: { id: "", name: "Anthropic", kind: "anthropic", baseUrl: "https://api.anthropic.com", model: "claude-opus-5-5", preset: "anthropic" },
    key: "sk-ant-demo-only",
  });
  await call("set_active_ai", { id: ollama.id });

  const browser = await webkit.launch();
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 2, colorScheme: theme });
  const shot = async (name) => {
    if (only.length && !only.includes(name)) return;
    await page.waitForTimeout(600);
    const png = `${out}/${name}-${theme}.png`;
    await page.screenshot({ path: png });
    execFileSync("cwebp", ["-quiet", "-q", "84", png, "-o", png.replace(/\.png$/, ".webp")]);
    unlinkSync(png);
    console.log(`${name}-${theme}.webp`);
  };

  await page.goto("http://localhost:1420");
  await page.evaluate(() => localStorage.clear());
  await page.reload();
  if (only.length && only.every((n) => LATER.includes(n))) {
    await later(page, shot).catch(async (e) => {
      await page.screenshot({ path: `${out}/failed-${theme}.png` });
      throw e;
    });
    await browser.close();
    return;
  }
  await page.getByText("Found on this computer").waitFor();
  await page.waitForTimeout(1800);
  await shot("welcome");

  // Connect to the demo store.
  await page.getByLabel("Connection URL").fill("postgres://kiyi:kiyi@localhost:55432/demo");
  await page.keyboard.press("Enter");
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Name", { exact: true }).fill("Acme Store");
  await dialog.getByRole("button", { name: "Save & connect" }).click();
  await page.waitForTimeout(1500);
  await shot("overview");

  await page.locator("main").getByRole("button", { name: /^customers/ }).first().click();
  await page.waitForTimeout(1500);
  await page.mouse.click(700, 260);
  await shot("data");

  // Ask AI.
  await page.mouse.click(700, 90);
  await page.getByLabel("Search rows or ask AI").fill("pro customers in germany who spent over 1k");
  await page.getByRole("button", { name: "Ask AI" }).click();
  await page.getByText("biggest spenders first").waitFor();
  await page.waitForTimeout(800);
  await shot("ai");

  // Structure.
  await page.getByRole("button", { name: "Structure", exact: true }).click();
  await page.waitForTimeout(800);
  await page.getByRole("row").filter({ hasText: "plan" }).first().click();
  await shot("structure");
  await page.keyboard.press("Escape");

  // A connection through an SSH bastion.
  await page.locator("aside").first().getByRole("button", { name: "Choose connection" }).click();
  await page.getByRole("button", { name: "New connection" }).first().click();
  await dialog.getByLabel("Connection URL").fill("postgres://kiyi:kiyi@postgres:5432/demo");
  await page.waitForTimeout(400);
  await dialog.getByLabel("Name", { exact: true }).fill("Acme Store (production)");
  await dialog.getByRole("radio", { name: "Production" }).click();
  await dialog.getByRole("radio", { name: "SSH", exact: true }).click();
  await dialog.getByLabel("SSH host").fill("127.0.0.1");
  await dialog.getByLabel("SSH port").fill("52222");
  await dialog.getByLabel("SSH user").fill("kiyi");
  await dialog.getByRole("radio", { name: "Password" }).first().click();
  await dialog.getByLabel("SSH password").fill("kiyi");
  await dialog.getByRole("button", { name: "Test connection" }).click();
  await dialog.getByText("Signed in to 127.0.0.1 as kiyi").waitFor({ timeout: 20000 });
  await dialog.evaluate((el) => el.closest("[class*=modal]")?.scrollTo(0, 10_000));
  await shot("connect");
  await dialog.getByRole("button", { name: "Close" }).click();

  // AI providers in Settings.
  await page.locator("aside").first().getByRole("button", { name: "Settings" }).click();
  await page.getByRole("button", { name: "AI", exact: true }).click();
  await page.waitForTimeout(600);
  await shot("settings-ai");
  await page.keyboard.press("Escape");

  // Ask a question, get a chart.
  await page.locator("aside").first().getByRole("button", { name: "Overview" }).click();
  await page.getByLabel("Ask a question").fill("monthly revenue over the last year");
  await page.keyboard.press("Enter");
  await page.getByText("refunds left out").waitFor({ timeout: 15000 });
  await page.waitForTimeout(1200);
  await shot("chart");

  // Move data: another company's CRM (PostgreSQL) into "My shop" (MySQL).
  const legacy = await call("parse_connection_url", { url: "postgres://kiyi:kiyi@localhost:55432/kiyi_legacy" });
  await call("save_connection", { config: { ...legacy.config, name: "Acme legacy CRM" }, password: "kiyi", tunnelSecret: null });
  const shop = await call("parse_connection_url", { url: "mysql://root:kiyi@localhost:53306/kiyi_move" });
  await call("save_connection", { config: { ...shop.config, name: "My shop", env: "local" }, password: "kiyi", tunnelSecret: null });
  await page.reload();
  await page.waitForTimeout(1500);
  await page.keyboard.press("Meta+k");
  await page.keyboard.type("My shop");
  await page.waitForTimeout(300);
  await page.keyboard.press("Enter");
  await page.locator("main").getByRole("button", { name: "Move data" }).click();
  await page.getByLabel("Move data from").selectOption({ label: "Acme legacy CRM" });
  await page.waitForTimeout(1500);
  await page.getByRole("button", { name: "Suggest a plan" }).click();
  await page.locator("nav").getByText("customers", { exact: true }).waitFor();
  await page.locator("nav").getByText("customers", { exact: true }).click();
  const status = page.locator("tr", { has: page.locator("text=status") }).first();
  await status.getByLabel("Add a step").selectOption("map");
  await status.getByLabel("Value", { exact: true }).first().fill("A");
  await status.getByLabel("Becomes").first().fill("active");
  await status.getByRole("button", { name: "Add a value" }).click();
  await status.getByLabel("Value", { exact: true }).nth(1).fill("P");
  await status.getByLabel("Becomes").nth(1).fill("passive");
  await status.getByRole("button", { name: /Change 2 values/ }).click();
  const orders = async (f) => {
    await page.locator("nav").getByText("orders", { exact: true }).click();
    await page.waitForTimeout(500);
    await f();
  };
  await orders(async () => {
    await page.locator("tr", { has: page.locator("text=ordered_at") }).getByLabel("Value from").selectOption("col:placed");
    const refunded = page.locator("tr", { has: page.locator("text=refunded") });
    await refunded.getByLabel("Value from").selectOption("col:status");
    await refunded.getByLabel("Add a step").selectOption("map");
    await refunded.getByLabel("Value", { exact: true }).first().fill("refunded");
    await refunded.getByLabel("Becomes").first().fill("yes");
    await refunded.getByLabel("Anything else").selectOption("value");
    await refunded.getByLabel("Other values become").fill("no");
  });
  await page.locator("nav").getByText("customers", { exact: true }).click();
  await page.getByRole("button", { name: "Check", exact: true }).click();
  await page.getByText("checked, ready to move").waitFor({ timeout: 20000 });
  await page.waitForTimeout(800);
  await shot("move");

  await later(page, shot);
  await browser.close();
}

/** Dashboards, editing a query's result, and data quality. */
async function later(page, shot) {
  for (const c of await call("list_connections")) await call("delete_connection", { id: c.id });
  const demo = await call("parse_connection_url", { url: "postgres://kiyi:kiyi@localhost:55432/demo" });
  const acme = await call("save_connection", { config: { ...demo.config, name: "Acme Store" }, password: "kiyi", tunnelSecret: null });
  const tile = (id, title, sql, chart, wide = false) => ({ id, title, sql, chart, wide });
  await call("save_user_data", {
    name: "dashboards",
    value: [
      {
        id: "acme-sales",
        connectionId: acme.id,
        name: "Sales overview",
        refreshMinutes: 15,
        updatedAt: Date.now(),
        tiles: [
          tile("t1", "Revenue this year", "SELECT sum(total) AS revenue FROM orders WHERE status <> 'refunded' AND placed_at >= date_trunc('year', now())", { kind: "number", x: null, y: ["revenue"] }),
          tile("t2", "Orders by status", "SELECT status, count(*) AS orders FROM orders GROUP BY 1 ORDER BY 2 DESC", { kind: "pie", x: "status", y: ["orders"] }),
          tile(
            "t3",
            "Revenue by month",
            "SELECT date_trunc('month', placed_at)::date AS month, sum(total) AS revenue FROM orders WHERE status <> 'refunded' AND placed_at >= date_trunc('month', now()) - interval '12 months' AND placed_at < date_trunc('month', now()) GROUP BY 1 ORDER BY 1",
            { kind: "line", x: "month", y: ["revenue"] },
            true,
          ),
          tile("t4", "Customers by plan", "SELECT plan, count(*) AS customers FROM customers GROUP BY 1 ORDER BY 1", { kind: "bar", x: "plan", y: ["customers"] }),
          tile("t5", "Top countries", "SELECT country, sum(lifetime_value) AS lifetime_value FROM customers GROUP BY 1 ORDER BY 2 DESC LIMIT 6", { kind: "bar", x: "country", y: ["lifetime_value"] }),
        ],
      },
    ],
  });
  await page.evaluate(() => localStorage.clear());
  await page.reload();
  await page.waitForTimeout(1500);
  await page.keyboard.press("Meta+k");
  await page.keyboard.type("Acme Store");
  await page.waitForTimeout(300);
  await page.keyboard.press("Enter");
  await page.waitForTimeout(1500);
  await page.locator("aside").getByText("Sales overview", { exact: true }).click();
  await page.locator("main section svg").nth(3).waitFor({ timeout: 15000 });
  await page.waitForTimeout(1200);
  await shot("dashboard");

  // A query's result, edited in place before Save.
  await page.keyboard.press("Meta+k");
  await page.keyboard.type("New SQL query");
  await page.keyboard.press("Enter");
  await page.waitForTimeout(500);
  await page.locator("main .cm-content").last().click();
  await page.keyboard.type("SELECT id, name, email, plan, lifetime_value\nFROM customers\nWHERE country = 'Germany'\nORDER BY lifetime_value DESC\nLIMIT 50");
  await page.locator("main").getByRole("button", { name: /^Run/ }).click();
  await page.getByText(/^Editable · customers/).waitFor();
  const grid = await page.locator("main [data-testid='data-grid-canvas']").last().boundingBox();
  // The grid is a canvas: find a column's x the way ResultGrid sizes columns (header and the first
  // 60 values, 7.3 px a character, 72 to 380 px), after the 32 px row numbers.
  const sql = await page.locator("main .cm-content").last().innerText();
  const events = await call("run_query", { connectionId: acme.id, queryId: "marketing-widths", sql });
  const cols = events.find((e) => e.columns)?.columns.map((c) => c.name) ?? [];
  const rows = events.flatMap((e) => e.rows ?? []);
  const widths = cols.map((name, i) => {
    let longest = name.length + 4;
    for (const r of rows.slice(0, 60)) longest = Math.max(longest, r[i] === null ? 4 : Math.min(String(r[i]).length, 60));
    return Math.round(Math.min(380, Math.max(72, longest * 7.3 + 28)));
  });
  const colX = (name) => {
    const i = cols.indexOf(name);
    return 32 + widths.slice(0, i).reduce((a, b) => a + b, 0) + widths[i] / 2;
  };
  const cell = async (row, column, value) => {
    await page.mouse.dblclick(grid.x + colX(column), grid.y + 30 + 26 * row + 13);
    const editor = page.locator("textarea.gdg-input");
    await editor.fill(value);
    await editor.press("Enter");
    await page.waitForTimeout(200);
  };
  await cell(1, "plan", "enterprise");
  await cell(5, "plan", "pro");
  await page.mouse.click(grid.x + grid.width - 40, grid.y + 30 + 26 * 8 + 13);
  await shot("results");

  // Data quality, on the sample database every Kiyi can open.
  const sample = await call("create_sample");
  await page.reload();
  await page.waitForTimeout(1500);
  await page.keyboard.press("Meta+k");
  await page.keyboard.type(sample.name);
  await page.waitForTimeout(300);
  await page.keyboard.press("Enter");
  await page.waitForTimeout(1200);
  await page.locator("aside").getByText("customers", { exact: true }).click();
  await page.waitForTimeout(800);
  await page.getByRole("button", { name: "Quality", exact: true }).click();
  await page.getByText("city spells the same value differently").waitFor({ timeout: 20000 });
  await page.waitForTimeout(800);
  await shot("quality");
}

try {
  for (const theme of ["dark", "light"]) await capture(theme);
} finally {
  await reset();
  stub.close();
}
