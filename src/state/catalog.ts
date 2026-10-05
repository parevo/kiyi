import { create } from "zustand";
import { ipc } from "../lib/ipc";
import type { ConnectionConfig, DriverInfo, TypeCategory } from "../lib/types";

interface CatalogState {
  drivers: DriverInfo[];
  load(): Promise<void>;
}

export const useCatalog = create<CatalogState>((set) => ({
  drivers: [],
  async load() {
    set({ drivers: await ipc.listDrivers() });
  },
}));

export function driverFor(c: Pick<ConnectionConfig, "kind" | "driver"> | undefined, drivers: DriverInfo[]) {
  if (!c) return undefined;
  return drivers.find((d) => d.id === c.driver) ?? drivers.find((d) => d.kind === c.kind);
}

export const CATEGORY_LABEL: Record<TypeCategory, string> = {
  text: "Metin",
  number: "Tam sayı",
  decimal: "Ondalık sayı",
  boolean: "Evet / Hayır",
  date: "Tarih",
  dateTime: "Tarih ve saat",
  time: "Saat",
  identifier: "Kimlik",
  json: "JSON",
  binary: "İkili veri",
  list: "Liste",
  other: "Diğer",
};

/** Category of a native column type such as `character varying(120)` or `int unsigned`. */
export function categorise(dataType: string, driver: DriverInfo | undefined): TypeCategory {
  const t = dataType.toLowerCase().trim();
  for (const [prefix, cat] of driver?.recognise ?? []) if (t.startsWith(prefix)) return cat;
  return "other";
}

/** The friendly name for a native type: the catalog label when it's an exact match. */
export function friendlyType(dataType: string, driver: DriverInfo | undefined): string {
  const exact = driver?.types.find((o) => o.sql.toLowerCase() === dataType.toLowerCase());
  return exact?.label ?? CATEGORY_LABEL[categorise(dataType, driver)];
}
