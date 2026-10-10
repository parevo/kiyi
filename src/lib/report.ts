import { ipc } from "./ipc";

/** Lines of the log to include; the whole log is a click away in Settings → About. */
const LOG_LINES = 40;

/**
 * Opens a new GitHub issue with the diagnostics filled in. Nothing is sent from Kiyi: the person
 * reads the issue in their browser and decides whether to submit it.
 */
export async function reportProblem(title: string, error?: string) {
  const diagnostics = await ipc.diagnostics().catch(() => "");
  const [head, ...rest] = diagnostics.split("\n\n");
  const log = rest.join("\n\n").split("\n").slice(-LOG_LINES).join("\n");
  const body = [
    "### What happened?",
    "",
    "<!-- What were you doing, and what did you expect instead? -->",
    "",
    ...(error ? ["### Error", "", "```", error.trim(), "```", ""] : []),
    "### Diagnostics",
    "",
    "<!-- Version, system and the end of Kiyi's log. It never contains passwords, keys or SQL text; check it before submitting. -->",
    "",
    "```",
    head ?? "",
    "",
    log,
    "```",
  ].join("\n");
  await ipc.openIssue(title, body);
}
