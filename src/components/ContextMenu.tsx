import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import s from "./ContextMenu.module.css";

export type MenuEntry =
  | { label: string; onSelect(): void; danger?: boolean; disabled?: boolean; shortcut?: string }
  | "separator";

export interface MenuState {
  x: number;
  y: number;
  items: MenuEntry[];
}

/** A right-click menu at the pointer. Keyboard: arrows move, Enter selects, Escape closes. */
export function ContextMenu({ menu, onClose }: { menu: MenuState | null; onClose(): void }) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ x: 0, y: 0 });

  useLayoutEffect(() => {
    if (!menu || !ref.current) return;
    const { width, height } = ref.current.getBoundingClientRect();
    setPos({
      x: Math.min(menu.x, window.innerWidth - width - 8),
      y: menu.y + height > window.innerHeight - 8 ? Math.max(8, menu.y - height) : menu.y,
    });
    ref.current.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
  }, [menu]);

  useEffect(() => {
    if (!menu) return;
    const close = (e: Event) => {
      if (e instanceof MouseEvent && ref.current?.contains(e.target as Node)) return;
      onClose();
    };
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") return onClose();
      if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
      e.preventDefault();
      const buttons = [...(ref.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? [])];
      const i = buttons.indexOf(document.activeElement as HTMLButtonElement);
      buttons[(i + (e.key === "ArrowDown" ? 1 : -1) + buttons.length) % buttons.length]?.focus();
    };
    window.addEventListener("mousedown", close, true);
    window.addEventListener("blur", close);
    window.addEventListener("keydown", key, true);
    return () => {
      window.removeEventListener("mousedown", close, true);
      window.removeEventListener("blur", close);
      window.removeEventListener("keydown", key, true);
    };
  }, [menu, onClose]);

  if (!menu) return null;
  return createPortal(
    <div ref={ref} className={s.menu} role="menu" style={{ left: pos.x || menu.x, top: pos.y || menu.y }}>
      {menu.items.map((item, i) =>
        item === "separator" ? (
          <div key={i} className={s.separator} />
        ) : (
          <button
            key={i}
            role="menuitem"
            className={s.item}
            data-danger={item.danger || undefined}
            disabled={item.disabled}
            onClick={() => {
              onClose();
              item.onSelect();
            }}
          >
            {item.label}
            {item.shortcut && <span className={s.shortcut}>{item.shortcut}</span>}
          </button>
        ),
      )}
    </div>,
    document.body,
  );
}
