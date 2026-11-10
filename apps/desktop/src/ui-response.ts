import { localizeBackendText, localizeBackendTextForLanguage, type InterfaceLanguage } from "./i18n.ts";
import { hasOwnedModelName } from "./settings-presentation.ts";

function record(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function fields(value: unknown, names: readonly string[], language?: InterfaceLanguage): unknown {
  if (!record(value)) return value;
  const result = { ...value };
  for (const name of names) if (typeof result[name] === "string") result[name] = language ? localizeBackendTextForLanguage(result[name], language) : localizeBackendText(result[name]);
  return result;
}

/** These are owned UI metadata/error channels. Content and machine status fields remain literal. */
export function localizeUiResponse(command: string, value: unknown): unknown {
  if (["app_info", "save_settings", "add_provider", "remove_provider", "set_key", "delete_model"].includes(command)
      && record(value) && record(value.settings) && Array.isArray(value.kinds)) {
    const language = value.interface_language === "nl" ? "nl" : "en";
    const result = fields(value, ["hotkey_error", "migration_warning"], language) as Record<string, unknown>;
    result.kinds = value.kinds.map(kind => {
      if (!record(kind)) return kind;
      return { ...fields(kind, ["label"], language) as object, availability: fields(kind.availability, ["reason"], language) };
    });
    if (record(value.capabilities)) result.capabilities = Object.fromEntries(Object.entries(value.capabilities).map(([key, capability]) => [key, fields(capability, ["reason"], language)]));
    return result;
  }
  if (command === "get_permissions" && Array.isArray(value)) return value.map(permission => fields(permission, ["label", "description", "detail", "reason"]));
  if (command === "local_models" && Array.isArray(value)) return value.map(model => {
    if (!record(model)) return model;
    const ownedName = hasOwnedModelName(model.id);
    return fields(model, ownedName ? ["name", "description"] : ["description"]);
  });
  if (command === "meeting_status" || command === "meeting_runtime_status") return fields(value, ["error", "audio_warning", "persistence_error", "transcription_warning", "terminal_error"]);
  if (command === "meeting_detail" && record(value)) return {
    ...value,
    retranscription_plan: fields(value.retranscription_plan, ["reason"]),
    retranscription_status: fields(value.retranscription_status, ["error", "warning"]),
  };
  if (["onboarding_readiness", "test_provider_task"].includes(command)) return fields(value, ["reason", "message"]);
  if (["open_dictation_session", "get_dictation_session", "toggle_dictation_session", "save_dictation_session"].includes(command)) return fields(value, ["error", "external_paste_reason"]);
  if (command === "dictation_state" && record(value) && value.state === "problem") return fields(value, ["title", "detail"]);
  return value;
}
