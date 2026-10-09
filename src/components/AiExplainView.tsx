import type { QueryExplanation } from "../lib/types";
import { AlertIcon, CloseIcon, SparklesIcon } from "./icons";
import { IconButton } from "./ui";
import s from "./PlanView.module.css";

/** A query explained in plain words by AI, in place of the results. */
export function AiExplainView({ explanation, onClose }: { explanation: QueryExplanation; onClose(): void }) {
  return (
    <div className={s.view}>
      <div className={s.header}>
        <div>
          <b>
            <SparklesIcon size={13} /> What this query does
          </b>
          <span className={s.sub}>written by AI from the SQL; check it before relying on it</span>
        </div>
        <IconButton label="Close" onPress={onClose}>
          <CloseIcon />
        </IconButton>
      </div>
      <div className={s.tree}>
        <p className="selectable" style={{ marginTop: 0 }}>
          {explanation.summary}
        </p>
        {explanation.steps.length > 0 && (
          <ol className="selectable">
            {explanation.steps.map((step, i) => (
              <li key={i}>{step}</li>
            ))}
          </ol>
        )}
        {explanation.warnings.map((w, i) => (
          <p key={i} className={s.warning}>
            <AlertIcon size={13} /> {w}
          </p>
        ))}
      </div>
    </div>
  );
}
