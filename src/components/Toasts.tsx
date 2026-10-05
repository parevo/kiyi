import { useToasts } from "../state/toasts";
import { AlertIcon, CheckIcon } from "./icons";
import s from "./Toasts.module.css";

export function Toasts() {
  const toasts = useToasts((st) => st.toasts);
  const dismiss = useToasts((st) => st.dismiss);
  return (
    <div className={s.stack} role="status" aria-live="polite">
      {toasts.map((t) => (
        <div key={t.id} className={s.toast} data-tone={t.tone} onClick={() => dismiss(t.id)}>
          {t.tone === "error" ? <AlertIcon /> : <CheckIcon />}
          <span className="selectable">{t.text}</span>
        </div>
      ))}
    </div>
  );
}
