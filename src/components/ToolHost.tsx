import { useEffect } from "react";
import { createPortal } from "react-dom";
import type { Tool } from "../state/ui";
import { useUi } from "../state/ui";
import { BackupView } from "./BackupView";
import { CompareView } from "./CompareView";
import { DiagramView } from "./DiagramView";
import { ObjectsView } from "./ObjectsView";

/** Full-workspace tools (diagram, objects, compare, backup), drawn over the tabs; Escape closes them. */
export function ToolHost({ tool }: { tool: Tool }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !document.querySelector("[role=dialog]")) useUi.getState().openTool(null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  const main = document.querySelector("main");
  if (!main) return null;
  return createPortal(tool === "diagram" ? <DiagramView /> : tool === "objects" ? <ObjectsView /> : tool === "backup" ? <BackupView /> : <CompareView />, main);
}
