import { relaunch } from "@tauri-apps/plugin-process";
import { useEffect, useRef, useState } from "react";
import { errorMessage, ipc } from "../lib/ipc";
import type { UpdateInfo } from "../lib/types";
import { useTabs } from "../state/tabs";
import { Spinner } from "./icons";
import { Button } from "./ui";
import s from "./UpdateNotice.module.css";

const CHECK_EVERY_MS = 4 * 60 * 60 * 1000;
const FIRST_CHECK_DELAY_MS = 5_000;

export function updateChannel(): "stable" | "beta" {
  try {
    return localStorage.getItem("kiyi.updateChannel") === "beta" ? "beta" : "stable";
  } catch {
    return "stable";
  }
}

/**
 * Checks quietly in the background, downloads without asking, and only installs when the
 * user clicks — so a running query or an open transaction is never cut off.
 */
export function UpdateNotice() {
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [ready, setReady] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [showNotes, setShowNotes] = useState(false);
  const busy = useRef(false);

  useEffect(() => {
    if (import.meta.env.DEV) return;
    const check = async () => {
      if (busy.current) return;
      busy.current = true;
      try {
        const found = await ipc.checkUpdate(updateChannel());
        if (found) {
          setUpdate(found);
          await ipc.downloadUpdate(() => {});
          setReady(true);
        }
      } catch (e) {
        console.warn("update check failed:", errorMessage(e));
      } finally {
        busy.current = false;
      }
    };
    const first = setTimeout(check, FIRST_CHECK_DELAY_MS);
    const every = setInterval(check, CHECK_EVERY_MS);
    return () => {
      clearTimeout(first);
      clearInterval(every);
    };
  }, []);

  if (!update || !ready) return null;

  const restart = async () => {
    const running = useTabs.getState().tabs.filter((t) => t.run?.status === "running").length;
    if (running > 0 && !confirm(`${running} sorgu hâlâ çalışıyor. Yeniden başlatınca iptal edilecek. Devam edilsin mi?`)) return;
    setInstalling(true);
    try {
      await ipc.installUpdate();
      await relaunch();
    } catch (e) {
      setInstalling(false);
      alert(`Güncelleme kurulamadı: ${errorMessage(e)}`);
    }
  };

  return (
    <div className={`${s.notice} ${update.critical ? s.critical : ""}`} role="status">
      {showNotes && update.notes && <div className={`${s.notes} selectable`}>{update.notes}</div>}
      <div className={s.text} onMouseEnter={() => setShowNotes(true)} onMouseLeave={() => setShowNotes(false)}>
        <span>Kıyı {update.version} hazır</span>
        <span className={s.sub}>{update.critical ? "Önemli güvenlik güncellemesi" : `Şu an ${update.currentVersion}`}</span>
      </div>
      <Button variant="primary" onPress={restart} isDisabled={installing}>
        {installing && <Spinner />}
        Yeniden başlat
      </Button>
    </div>
  );
}
