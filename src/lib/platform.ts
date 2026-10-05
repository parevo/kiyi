/** macOS uses ⌘ for shortcuts; Windows and Linux use Ctrl. */
export const isMac = /Mac|iPhone|iPad/.test(navigator.userAgent);

/** The platform's command modifier is held. */
export const isMod = (e: { metaKey: boolean; ctrlKey: boolean }) => (isMac ? e.metaKey : e.ctrlKey);

/** A shortcut label: `kbd("S")` is "⌘S" on macOS and "Ctrl+S" elsewhere. */
export const kbd = (key: string) => (isMac ? `⌘${key}` : `Ctrl+${key}`);
