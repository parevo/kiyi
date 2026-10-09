import { useEffect, useState } from "react";
import { Dialog, Heading, Modal, ModalOverlay } from "react-aria-components";
import { CloseIcon, Spinner } from "./icons";
import { Button, Field, IconButton } from "./ui";
import s from "./dialog.module.css";

export interface PromptField {
  label: string;
  value?: string;
  placeholder?: string;
  mono?: boolean;
  /** A NULL toggle: when allowed, the user can choose "no value" instead of text. */
  nullable?: boolean;
}

export interface PromptRequest {
  title: string;
  subtitle?: string;
  help?: string;
  fields: PromptField[];
  action: string;
  /** Values in field order; `null` where the user chose NULL. Throwing shows the message. */
  run(values: (string | null)[]): Promise<void> | void;
}

/** A small form in a dialog: "Set 12 cells to…", "Replace in column…", "Save view as…". */
export function PromptDialog({ request, onClose }: { request: PromptRequest | null; onClose(): void }) {
  const [values, setValues] = useState<(string | null)[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setValues(request?.fields.map((f) => f.value ?? "") ?? []);
    setBusy(false);
    setError(null);
  }, [request]);

  if (!request) return null;
  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      await request.run(values);
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : typeof e === "object" && e && "message" in e ? String(e.message) : String(e));
      setBusy(false);
    }
  };

  return (
    <ModalOverlay isOpen onOpenChange={(open) => !open && !busy && onClose()} isDismissable={!busy} className={s.overlay}>
      <Modal className={`${s.modal} ${s.small}`}>
        <Dialog className={s.dialog}>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              if (!busy) submit();
            }}
          >
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
              {request.fields.map((f, i) => (
                <div key={i}>
                  <Field
                    label={f.label}
                    mono={f.mono}
                    value={values[i] ?? ""}
                    placeholder={values[i] === null ? "NULL" : f.placeholder}
                    autoFocus={i === 0}
                    onChange={(v) => setValues((cur) => cur.map((x, j) => (j === i ? v : x)))}
                  />
                  {f.nullable && (
                    <label className={s.subtitle}>
                      <input type="checkbox" checked={values[i] === null} onChange={(e) => setValues((cur) => cur.map((x, j) => (j === i ? (e.target.checked ? null : "") : x)))} /> Set to
                      NULL (no value)
                    </label>
                  )}
                </div>
              ))}
              {request.help && <p className={s.subtitle}>{request.help}</p>}
              {error && (
                <p className={s.subtitle} role="alert" style={{ color: "var(--danger)" }}>
                  {error}
                </p>
              )}
            </div>
            <div className={s.footer}>
              <div className={s.footerRight}>
                <Button onPress={onClose} isDisabled={busy}>
                  Cancel
                </Button>
                <Button type="submit" variant="primary" isDisabled={busy}>
                  {busy && <Spinner />}
                  {request.action}
                </Button>
              </div>
            </div>
          </form>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
