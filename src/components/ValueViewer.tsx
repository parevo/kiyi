import { useEffect, useMemo, useState } from "react";
import { Dialog, Heading, Modal, ModalOverlay } from "react-aria-components";
import { formatSize, hexDump, hexToBytes, sniff } from "../lib/binary";
import { toast } from "../state/toasts";
import { CloseIcon, CopyIcon } from "./icons";
import { Button, IconButton, Segmented } from "./ui";
import d from "./dialog.module.css";
import s from "./ValueViewer.module.css";

/** Bytes shown in the hex dump; past that the size says how much more there is. */
const DUMP_LIMIT = 16 * 1024;

export interface ViewedValue {
  column: string;
  kind: "json" | "binary";
  value: string;
}

/** A closer look at one value: JSON as a tree, binary as an image preview or a hex dump. */
export function ValueViewer({ viewed, onClose }: { viewed: ViewedValue | null; onClose(): void }) {
  if (!viewed) return null;
  return (
    <ModalOverlay isOpen onOpenChange={(open) => !open && onClose()} isDismissable className={d.overlay}>
      <Modal className={`${d.modal} ${s.wide}`}>
        <Dialog className={d.dialog}>
          <div className={d.header}>
            <div>
              <Heading slot="title" className={d.title}>
                {viewed.column}
              </Heading>
              <p className={d.subtitle}>{viewed.kind === "json" ? "JSON" : "Binary"}</p>
            </div>
            <IconButton label="Close" onPress={onClose}>
              <CloseIcon />
            </IconButton>
          </div>
          {viewed.kind === "json" ? <JsonBody value={viewed.value} /> : <BinaryBody value={viewed.value} />}
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}

const copy = (text: string) => navigator.clipboard.writeText(text).then(() => toast.success("Copied"), () => toast.error("Couldn't copy"));

function JsonBody({ value }: { value: string }) {
  const parsed = useMemo(() => {
    try {
      return { ok: true as const, data: JSON.parse(value) as unknown };
    } catch {
      return { ok: false as const };
    }
  }, [value]);
  const [mode, setMode] = useState<"tree" | "text">(parsed.ok ? "tree" : "text");
  const pretty = parsed.ok ? JSON.stringify(parsed.data, null, 2) : value;
  return (
    <>
      <div className={d.body}>
        {!parsed.ok && <p className={d.subtitle}>This value isn't valid JSON, so it's shown as text.</p>}
        {parsed.ok && (
          <Segmented<"tree" | "text">
            label="Show as"
            value={mode}
            onChange={setMode}
            options={[
              { value: "tree", label: "Tree" },
              { value: "text", label: "Text" },
            ]}
          />
        )}
        <div className={`${s.box} selectable`}>{mode === "tree" && parsed.ok ? <JsonNode value={parsed.data} depth={0} /> : <pre className={s.pre}>{pretty}</pre>}</div>
      </div>
      <div className={d.footer}>
        <span className={d.subtitle}>{formatSize(new TextEncoder().encode(value).length)}</span>
        <Button onPress={() => copy(pretty)}>
          <CopyIcon size={14} /> Copy
        </Button>
      </div>
    </>
  );
}

function JsonNode({ value, name, depth }: { value: unknown; name?: string; depth: number }) {
  const label = name !== undefined && <span className={s.key}>{name}: </span>;
  if (value === null || typeof value !== "object") {
    const cls = value === null ? s.null : typeof value === "string" ? s.string : typeof value === "number" ? s.number : s.bool;
    return (
      <div className={s.leaf}>
        {label}
        <span className={cls}>{typeof value === "string" ? JSON.stringify(value) : String(value)}</span>
      </div>
    );
  }
  const entries: [string, unknown][] = Array.isArray(value) ? value.map((v, i) => [String(i), v]) : Object.entries(value);
  const summary = Array.isArray(value) ? `[${entries.length} ${entries.length === 1 ? "item" : "items"}]` : `{${entries.length} ${entries.length === 1 ? "key" : "keys"}}`;
  return (
    <details className={s.node} open={depth < 2}>
      <summary>
        {label}
        <span className={s.summary}>{summary}</span>
      </summary>
      <div className={s.children}>
        {entries.map(([k, v]) => (
          <JsonNode key={k} name={k} value={v} depth={depth + 1} />
        ))}
      </div>
    </details>
  );
}

function BinaryBody({ value }: { value: string }) {
  const bytes = useMemo(() => hexToBytes(value), [value]);
  const kind = bytes ? sniff(bytes) : null;
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    if (!bytes || !kind?.mime) return;
    const u = URL.createObjectURL(new Blob([bytes as BlobPart], { type: kind.mime }));
    setUrl(u);
    return () => URL.revokeObjectURL(u);
  }, [bytes, kind?.mime]);

  if (!bytes) {
    return (
      <div className={d.body}>
        <pre className={`${s.pre} ${s.box} selectable`}>{value}</pre>
      </div>
    );
  }
  return (
    <>
      <div className={d.body}>
        <p className={d.subtitle}>
          {formatSize(bytes.length)}
          {kind && ` · probably a ${kind.type.charAt(0).toLowerCase()}${kind.type.slice(1)}`}
        </p>
        {url && <img className={s.image} src={url} alt="Preview of the stored image" />}
        {kind?.type === "Plain text" ? (
          <pre className={`${s.pre} ${s.box} selectable`}>{new TextDecoder().decode(bytes.subarray(0, DUMP_LIMIT))}</pre>
        ) : (
          <pre className={`${s.pre} ${s.box} ${s.dump} selectable`}>{hexDump(bytes, DUMP_LIMIT)}</pre>
        )}
        {bytes.length > DUMP_LIMIT && <p className={d.subtitle}>Showing the first {formatSize(DUMP_LIMIT)}.</p>}
      </div>
      <div className={d.footer}>
        <span />
        <Button onPress={() => copy(value)}>
          <CopyIcon size={14} /> Copy as hex
        </Button>
      </div>
    </>
  );
}
