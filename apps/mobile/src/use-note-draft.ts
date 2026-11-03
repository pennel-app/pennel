import { useEffect, useMemo, useRef, useSyncExternalStore } from 'react';
import { NoteSession } from './note-session';
import { useRuntime } from './runtime';

export function useNoteDraft(id: string | null | undefined, remote: string, sessionKey = id) {
  const { run } = useRuntime();
  const target = useRef(id);
  target.current = id;
  // A remote refresh must update the session without replacing its pending edits or in-flight save.
  const session = useMemo(() => new NoteSession(remote, async (notes, expected_notes, insight_receipt) => {
    const meetingId = target.current;
    if (!meetingId) return false;
    return (await run(`notes-${meetingId}`, 'meetings.notes', { id: meetingId, notes, expected_notes, ...(insight_receipt ? { insight_receipt } : {}) })) !== undefined;
  }), [sessionKey, run]);
  const state = useSyncExternalStore(session.subscribe, session.getSnapshot, session.getSnapshot);
  useEffect(() => session.adoptRemote(remote), [remote, session]);
  useEffect(() => {
    if (!state.dirty || state.conflict || state.failed || state.saving) return;
    const timer = setTimeout(() => void session.save(), 500);
    return () => clearTimeout(timer);
  }, [session, state]);
  return { ...state, setDraft: session.edit, save: session.save, discard: session.discard, getCurrent: session.getSnapshot,
    stageInsightInsertion: session.stageInsightInsertion, retarget: (meetingId: string) => { target.current = meetingId; } };
}
