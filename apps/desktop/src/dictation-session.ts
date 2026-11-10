import { api, listenToWindow, type DictationSessionSnapshot } from "./api";
import { OwnedSessionLifecycle } from "./dictation-session-state";

export class OwnedDictationSession extends OwnedSessionLifecycle {
  constructor(kind: DictationSessionSnapshot["kind"], update: (snapshot: DictationSessionSnapshot) => void) {
    super(kind, update, {
      open: api.openDictationSession,
      get: api.getDictationSession,
      toggle: api.toggleDictationSession,
      cancel: api.cancelDictationSession,
      close: api.closeDictationSession,
      leave: (id) => api.closeDictationSession(id, true),
      subscribe: (handler) => listenToWindow<DictationSessionSnapshot>("dictation-session", (event) => handler(event.payload)),
    });
  }
}
