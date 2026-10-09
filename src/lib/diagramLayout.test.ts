import { describe, expect, it } from "vitest";
import { layout, type DiagramTable } from "./diagramLayout";

const t = (id: string): DiagramTable => ({ id, name: id, columns: [{ name: "id", type: "int", pk: true, fk: false }], hidden: 0 });

describe("diagram layout", () => {
  it("puts referenced tables left of the tables pointing at them", () => {
    const { nodes } = layout(
      [t("order_items"), t("orders"), t("customers"), t("products"), t("settings")],
      [
        { from: "orders", fromColumn: "customer_id", to: "customers", toColumn: "id" },
        { from: "order_items", fromColumn: "order_id", to: "orders", toColumn: "id" },
        { from: "order_items", fromColumn: "product_id", to: "products", toColumn: "id" },
      ],
    );
    const x = (id: string) => nodes.find((n) => n.id === id)!.x;
    expect(x("customers")).toBeLessThan(x("orders"));
    expect(x("orders")).toBeLessThan(x("order_items"));
    expect(x("products")).toBeLessThan(x("order_items"));
    // An unrelated table goes below the connected ones.
    const settings = nodes.find((n) => n.id === "settings")!;
    expect(settings.y).toBeGreaterThan(Math.max(...nodes.filter((n) => n.id !== "settings").map((n) => n.y)));
  });

  it("survives reference cycles and self references", () => {
    const { nodes } = layout([t("a"), t("b")], [
      { from: "a", fromColumn: "b_id", to: "b", toColumn: "id" },
      { from: "b", fromColumn: "a_id", to: "a", toColumn: "id" },
      { from: "a", fromColumn: "parent_id", to: "a", toColumn: "id" },
    ]);
    expect(nodes).toHaveLength(2);
  });
});
