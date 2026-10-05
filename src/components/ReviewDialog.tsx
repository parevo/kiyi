import { useEffect, useState } from "react";
import { Dialog, Heading, Input, Modal, ModalOverlay, TextField } from "react-aria-components";
import { HighlightedSql, isDestructive } from "../lib/highlight";
import type { DbKind, EnvTag, ErrorInfo } from "../lib/types";
import { AlertIcon, CloseIcon, Spinner } from "./icons";
import { Button, IconButton } from "./ui";
import s from "./dialog.module.css";
import ui from "./ui.module.css";

export interface ReviewRequest {
  title: string;
  subtitle?: string;
  statements: string[];
  /** Label of the apply button, e.g. "Değişiklikleri uygula". */
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

  useEffect(() => {
    setBusy(false);
    setError(null);
    setTyped("");
  }, [request]);

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
      const info = e as ErrorInfo;
      setError(typeof e === "string" ? { message: e, code: null, position: null } : info);
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
            <IconButton label="Kapat" onPress={onClose} isDisabled={busy}>
              <CloseIcon />
            </IconButton>
          </div>

          <div className={s.body}>
            {destructive > 0 && (
              <div className={`${s.notice} ${s.danger}`}>
                <AlertIcon />
                <span>
                  {destructive} ifade veri ya da yapı siliyor.
                  {env === "production" && " Bu bir production bağlantısı."}
                </span>
              </div>
            )}
            {request.nonTransactional && request.statements.length > 1 && (
              <div className={s.notice}>
                <AlertIcon />
                <span>MySQL yapı değişikliklerini geri alamaz. Bir adım hata verirse önceki adımlar uygulanmış kalır.</span>
              </div>
            )}

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

            {error && failed === null && (
              <div className={`${s.notice} ${s.danger}`} role="alert">
                <AlertIcon />
                <span className="selectable">{error.message}</span>
              </div>
            )}
            {error && failed !== null && !request.nonTransactional && (
              <div className={s.notice}>
                <AlertIcon />
                <span>Hiçbir değişiklik uygulanmadı, hepsi geri alındı.</span>
              </div>
            )}

            {needsWord && (
              <TextField className={s.confirmInput} value={typed} onChange={setTyped} aria-label="Onay">
                <span>
                  Devam etmek için <b>{request.confirmWord}</b> yaz
                </span>
                <Input className={`${ui.input} ${ui.mono}`} autoFocus spellCheck={false} />
              </TextField>
            )}
          </div>

          <div className={s.footer}>
            <Button variant="ghost" onPress={() => navigator.clipboard.writeText(script)}>
              SQL'i kopyala
            </Button>
            {onOpenInEditor && (
              <Button
                variant="ghost"
                onPress={() => {
                  onOpenInEditor(script);
                  onClose();
                }}
              >
                Editörde aç
              </Button>
            )}
            <div className={s.footerRight}>
              <Button onPress={onClose} isDisabled={busy}>
                Vazgeç
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
