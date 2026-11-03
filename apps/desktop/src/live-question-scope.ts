import type { LiveQuestionSnapshot } from "./api.ts";
import { t } from "./i18n.ts";

export function liveQuestionMetadata(snapshot: LiveQuestionSnapshot, time: string): string {
  const selection = snapshot.selection;
  const scope = selection?.scope === "selected_excerpts"
    ? t("meetings.workspace_selected_cutoff", { time, included: selection.included_sources, available: selection.available_sources })
    : t("meetings.workspace_cutoff", { time });
  return scope
    + (selection?.history_reduced ? ` · ${t("meetings.workspace_history_reduced", { included: selection.included_history_turns, available: selection.available_history_turns })}` : "")
    + (snapshot.pending_transcription ? ` · ${t("meetings.workspace_pending", { count: snapshot.pending_transcription })}` : "");
}
