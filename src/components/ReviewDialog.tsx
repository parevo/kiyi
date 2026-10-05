import { useEffect, useState } from "react";
import { Dialog, Heading, Input, Modal, ModalOverlay, TextField } from "react-aria-components";
import { HighlightedSql, isDestructive } from "../lib/highlight";
import type { DbKind, EnvTag, ErrorInfo } from "../lib/types";
import { useSettings } from "../state/settings";
import { AlertIcon, CloseIcon, Spinner } from "./icons";
import { Button, IconButton } from "./ui";
import s from "./dialog.module.css";
import ui from "./ui.module.css";

export interface ReviewRequest {
  title: string;
  subtitle?: string;
  /** Plain-language list of what will happen, shown instead of SQL. */
  summary: { text: string; danger?: boolean }[];
  statements: string[];
  /** Label of the apply button, e.g. "Apply changes". */
  action: string;
  /** Must be typed to enable the button on production when anything destructive is included. */
  confirmWord?: string;
  /** MySQL DDL can't be rolled back; say so. */
  nonTransactional?: boolean;
  run(): Promise<void>;
}

/**
 * Shows exactly the SQL that will run, flags destructive statements, and pins a failure to
 * the statement that caused it.
 */
export function ReviewDialog({
  request,
  kind,
  env,
  onClose,
  onOpenInEditor,
}: {
  request: ReviewRequest | null;
  kind: DbKind;
  env: EnvTag;
  onClose(): void;
  onOpenInEditor?(sql: string): void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<ErrorInfo | null>(null);
  const [typed, setTyped] = useState("");
  const developerMode = useSettings((st) => st.developerMode);
  const [showSql, setShowSql] = useState(developerMode);

  useEffect(() => {
    setBusy(false);
    setError(null);
    setTyped("");
    setShowSql(developerMode);
  }, [request, developerMode]);

  if (!request) return null;
  const destructive = request.statements.filter(isDestructive).length;
  const needsWord = env === "production" && destructive > 0 && !!request.confirmWord;
  const failed = error?.statementIndex ?? null;
  const script = request.statements.map((st) => st + ";").join("\n");

  const apply = async () => {
    setBusy(true);
    setError(null);
    try {
      await request.run();
      onClose();
    } catch (e) {
      const info = typeof e === "string" ? { message: e, code: null, position: null } : (e as ErrorInfo);
      setError(info);
      // Point at the statement that failed.
      if (info.statementIndex != null) setShowSql(true);
      setBusy(false);
    }
  };

  return (
    <ModalOverlay isOpen onOpenChange={(open) => !open && !busy && onClose()} isDismissable={!busy} className={s.overlay}>
      <Modal className={s.modal}>
        <Dialog className={s.dialog}>
          <div className={s.header}>
            <div>
              <Heading slot="title" className={s.title}>
                {request.title}
              </Heading>
              {request.subtitle && <p className={s.subtitle}>{request.subtitle}</p>}
            </div>
            <IconButton label="Close" onPress={onClose} isDisabled={busy}>
              <CloseIcon />
            </IconButton>
          </div>

          <div className={s.body}>
            <ul className={s.summary}>
              {request.summary.map((item, i) => (
                <li key={i} data-danger={item.danger || undefined}>
                  {item.text}
                </li>
              ))}
            </ul>
            {env === "production" && destructive > 0 && (
              <div className={`${s.notice} ${s.danger}`}>
                <AlertIcon />
                <span>This is a production connection. Deleted data can't be recovered.</span>
              </div>
            )}
            {request.nonTransactional && request.statements.length > 1 && (
              <div className={s.notice}>
                <AlertIcon />
                <span>This database can't roll back structure changes. If a step fails, the steps before it stay applied.</span>
              </div>
            )}

            <button className={s.disclosure} onClick={() => setShowSql((v) => !v)} aria-expanded={showSql}>
              {showSql ? "▾" : "▸"} Technical details ({request.statements.length} SQL {request.statements.length === 1 ? "statement" : "statements"})
            </button>
            {showSql && (
              <ol className={`${s.statements} selectable`}>
                {request.statements.map((st, i) => (
                  <li
                    key={i}
                    className={s.statement}
                    data-destructive={isDestructive(st) || undefined}
                    data-failed={failed === i || undefined}
                    // Without a transaction, statements before the failure were applied.
                    data-done={(request.nonTransactional && failed !== null && i < failed) || undefined}
                  >
                    <pre className={s.code}>
                      <HighlightedSql sql={st} kind={kind} />
                    </pre>
                    {failed === i && <div className={s.stmtError}>{error?.message}</div>}
                  </li>
                ))}
              </ol>
            )}

            {error && (failed === null || !showSql) && (
              <div className={`${s.notice} ${s.danger}`} role="alert">
                <AlertIcon />
                <span className="selectable">{error.message}</span>
              </div>
            )}
            {error && failed !== null && !request.nonTransactional && (
              <div className={s.notice}>
                <AlertIcon />
                <span>Nothing was changed; everything was rolled back.</span>
              </div>
            )}

            {needsWord && (
              <TextField className={s.confirmInput} value={typed} onChange={setTyped} aria-label="Confirmation">
                <span>
                  Type <b>{request.confirmWord}</b> to continue
                </span>
                <Input className={`${ui.input} ${ui.mono}`} autoFocus spellCheck={false} />
              </TextField>
            )}
          </div>

          <div className={s.footer}>
            {showSql && (
              <Button variant="ghost" onPress={() => navigator.clipboard.writeText(script)}>
                Copy SQL
              </Button>
            )}
            {showSql && onOpenInEditor && (
              <Button
                variant="ghost"
                onPress={() => {
                  onOpenInEditor(script);
                  onClose();
                }}
              >
                Open in SQL editor
              </Button>
            )}
            <div className={s.footerRight}>
              <Button onPress={onClose} isDisabled={busy}>
                Cancel
              </Button>
              <Button
                variant="primary"
                className={destructive > 0 ? s.dangerButton : undefined}
                onPress={apply}
                isDisabled={busy || (needsWord && typed.trim() !== request.confirmWord)}
                autoFocus={!needsWord}
              >
                {busy && <Spinner />}
                {request.action}
              </Button>
            </div>
          </div>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
