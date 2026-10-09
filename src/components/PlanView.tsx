import type { PlanNode } from "../lib/types";
import { AlertIcon, CloseIcon } from "./icons";
import { IconButton } from "./ui";
import s from "./PlanView.module.css";

const fmt = new Intl.NumberFormat("en-US", { maximumFractionDigits: 1, notation: "compact" });

function count(node: PlanNode): number {
  return (node.warning ? 1 : 0) + node.children.reduce((n, c) => n + count(c), 0);
}

function Step({ node, depth }: { node: PlanNode; depth: number }) {
  return (
    <li className={s.step} style={{ ["--depth" as string]: depth }}>
      <div className={s.row} data-warning={node.warning ? true : undefined}>
        <span className={s.label}>{node.label}</span>
        <span className={s.figures}>
          {node.rows !== null && <span title="Estimated rows">~{fmt.format(node.rows)} rows</span>}
          {node.cost !== null && <span title="Estimated cost, in the database's own units">cost {fmt.format(node.cost)}</span>}
        </span>
      </div>
      {node.detail && <pre className={`${s.detail} selectable`}>{node.detail}</pre>}
      {node.warning && (
        <p className={s.warning}>
          <AlertIcon size={13} /> {node.warning}
        </p>
      )}
      {node.children.length > 0 && (
        <ul className={s.children}>
          {node.children.map((c, i) => (
            <Step key={i} node={c} depth={depth + 1} />
          ))}
        </ul>
      )}
    </li>
  );
}

/** A query plan as an indented list of steps, read top-down: each step uses the ones below it. */
export function PlanView({ plan, onClose }: { plan: PlanNode; onClose(): void }) {
  const warnings = count(plan);
  return (
    <div className={s.view}>
      <div className={s.header}>
        <div>
          <b>Query plan</b>
          <span className={s.sub}>
            {warnings ? `${warnings} ${warnings === 1 ? "step looks" : "steps look"} slow` : "No full table scans"} · estimates only, nothing was run
          </span>
        </div>
        <IconButton label="Close plan" onPress={onClose}>
          <CloseIcon />
        </IconButton>
      </div>
      <ul className={s.tree}>
        <Step node={plan} depth={0} />
      </ul>
    </div>
  );
}
