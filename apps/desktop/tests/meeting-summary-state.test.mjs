import assert from "node:assert/strict";
import test from "node:test";
import { SummaryDraft } from "../src/meeting-summary-state.ts";

const document = (markdown = "Saved summary", revision = "r1", source_revision = "s1") => ({ meeting_uid: "uid-one", markdown, revision, source_revision, edited: false, outdated: false });

test("background summary completion cannot replace typed text or a focused clean editor", () => {
  const draft = new SummaryDraft();
  assert.equal(draft.receive(document()), true);
  draft.edit("My current writing");
  assert.equal(draft.receive(document("Generated remotely", "r2")), false);
  assert.equal(draft.value, "My current writing");
  assert.equal(draft.document.revision, "r1");
  assert.equal(draft.remote.revision, "r2");
  draft.useRemote();
  assert.equal(draft.value, "Generated remotely");
  assert.equal(draft.receive(document("Another generated summary", "r3"), true), false);
  assert.equal(draft.value, "Generated remotely");
});

test("record attachment metadata follows a same-body refresh without replacing the acknowledged text", () => {
  const draft = new SummaryDraft();
  draft.receive({ ...document(), record: { body_revision: "attached" } });
  assert.equal(draft.receive({ ...document(), record: null }), false);
  assert.equal(draft.document.record, null);
  assert.equal(draft.value, "Saved summary");
});

test("same-body presentation metadata defers across protected reading and applies after release", () => {
  for (const attached of [true, false]) {
    const record = { body_revision: "attached", structure: { lead: "Saved summary" } };
    const draft = new SummaryDraft();
    draft.receive({ ...document(), record: attached ? record : null });
    draft.receive({ ...document(), outdated: true, record: attached ? null : record }, true);
    assert.deepEqual(draft.document.record, attached ? record : null);
    assert.equal(draft.document.outdated, true, "Harmless status metadata was not acknowledged.");
    assert.equal(draft.remote, null, "A presentation-only change became a body conflict.");
    assert.ok(draft.deferredPresentation);
    assert.equal(draft.applyPresentation(true), false);
    assert.equal(draft.applyPresentation(), true);
    assert.deepEqual(draft.document.record, attached ? null : record);
    assert.equal(draft.value, "Saved summary");
    assert.equal(draft.deferredPresentation, null);
  }
});

test("an acknowledged write supersedes deferred presentation metadata for its old revision", () => {
  const draft = new SummaryDraft();
  draft.receive({ ...document(), record: { body_revision: "attached" } });
  draft.receive({ ...document(), record: null }, true);
  assert.ok(draft.deferredPresentation);
  draft.value = "Acknowledged action state";
  draft.acknowledge(document(draft.value, "r2"), draft.value);
  assert.equal(draft.deferredPresentation, null);
  assert.equal(draft.applyPresentation(), false);
  assert.equal(draft.document.revision, "r2");
});

test("source changes never silently advance the expected revision of an active or dirty draft", () => {
  for (const protectedInput of [true, false]) {
    const draft = new SummaryDraft(); draft.receive(document());
    if (!protectedInput) draft.edit("A summary based on the old transcript");
    assert.equal(draft.receive({ ...document("Saved summary", "r1", "s2"), outdated: true }, protectedInput), false);
    assert.equal(draft.document.source_revision, "s1");
    assert.equal(draft.remote.source_revision, "s2");
    assert.equal(draft.keepEditAgainstRemote(), true);
    assert.equal(draft.document.source_revision, "s2");
    assert.equal(draft.value, protectedInput ? "Saved summary" : "A summary based on the old transcript");
  }
});

test("a generated replacement is reviewable and later typing or source changes invalidate acceptance", () => {
  const draft = new SummaryDraft(); draft.receive(document());
  draft.stage(document("New candidate"), draft.editRevision);
  assert.equal(draft.value, "Saved summary");
  assert.equal(draft.canAccept, true);
  draft.edit("Keep this later typing");
  assert.equal(draft.canAccept, false);
  draft.stage(document("Newer candidate"), draft.editRevision);
  assert.equal(draft.canAccept, true);
  draft.receive(document("Saved summary", "r1", "s2"));
  assert.equal(draft.canAccept, false);
  assert.equal(draft.value, "Keep this later typing");
});

test("save acknowledgements advance durable revision while retaining typing done during the write", () => {
  const draft = new SummaryDraft(); draft.receive(document());
  draft.edit("First edit"); const target = draft.value;
  draft.edit("Newer edit while saving");
  draft.acknowledge(document(target, "r2"), target);
  assert.equal(draft.document.revision, "r2");
  assert.equal(draft.value, "Newer edit while saving");
  assert.equal(draft.dirty, true);
  draft.acknowledge(document(draft.value, "r3"), draft.value);
  assert.equal(draft.dirty, false);
});

test("explicit conflict recovery keeps the edit and dismisses the obsolete candidate", () => {
  const draft = new SummaryDraft(); draft.receive(document()); draft.edit("Unsent edit");
  draft.stage(document("Candidate"), draft.editRevision);
  draft.receive(document("Other window", "r2", "s2"));
  assert.equal(draft.keepEditAgainstRemote(), true);
  assert.equal(draft.document.revision, "r2");
  assert.equal(draft.value, "Unsent edit");
  assert.equal(draft.dirty, true);
  assert.equal(draft.candidate, null);
});

test("manual review is available only for the exact clean saved outdated summary", () => {
  const draft = new SummaryDraft(); draft.receive({ ...document(), outdated: true });
  assert.equal(draft.canMarkReviewed, true);
  draft.edit("Not yet saved"); assert.equal(draft.canMarkReviewed, false);
  draft.edit("Saved summary"); assert.equal(draft.canMarkReviewed, true);
  draft.pending = true; assert.equal(draft.canMarkReviewed, false); draft.pending = false;
  draft.receive({ ...document("Saved summary", "r1", "changed-notes-or-transcript"), outdated: true }, true);
  assert.equal(draft.document.source_revision, "s1");
  assert.equal(draft.canMarkReviewed, false);
  draft.keepEditAgainstRemote(); assert.equal(draft.canMarkReviewed, true);
  draft.acknowledge(document("Saved summary", "r1", "changed-notes-or-transcript"), "Saved summary");
  assert.equal(draft.canMarkReviewed, false);
});

test("our metadata-only acknowledgement preserves a reviewable candidate and newer typing", () => {
  const draft = new SummaryDraft(); draft.receive(document()); draft.stage(document("Candidate"), draft.editRevision); draft.error = "Earlier generation failed";
  assert.equal(draft.acknowledgeMetadata(document("Saved summary", "r2")), true);
  assert.equal(draft.value, "Saved summary"); assert.equal(draft.candidate.revision, "r2"); assert.equal(draft.canAccept, true); assert.equal(draft.error, "Earlier generation failed");
  draft.edit("Later writer text"); const revision = draft.editRevision;
  assert.equal(draft.acknowledgeMetadata(document("Saved summary", "r3")), true);
  assert.equal(draft.value, "Later writer text"); assert.equal(draft.dirty, true); assert.equal(draft.editRevision, revision); assert.equal(draft.canAccept, false);
});
test("our metadata acknowledgement supersedes deferred record presentation", () => {
  for (const revision of ["r1", "r2"]) {
    const record = { body_revision: "attached", structure: { lead: "Saved summary" } };
    const draft = new SummaryDraft();
    draft.receive({ ...document(), record });
    draft.receive({ ...document(), record: null }, true);
    assert.ok(draft.deferredPresentation);
    assert.equal(draft.acknowledgeMetadata({ ...document("Saved summary", revision), record }), true);
    assert.equal(draft.deferredPresentation, null);
    assert.equal(draft.applyPresentation(), false);
    assert.deepEqual(draft.document.record, record);
    assert.equal(draft.document.revision, revision);
  }
});
test("metadata acknowledgement cannot rebase changed content, sources or meeting identity", () => {
  for (const changed of [{ ...document("Saved summary", "r2"), meeting_uid: "other" }, document("Different body", "r2"), document("Saved summary", "r2", "s2")]) {
    const draft = new SummaryDraft(); draft.receive(document()); draft.edit("Keep my typing");
    assert.equal(draft.acknowledgeMetadata(changed), false);
    assert.equal(draft.document.revision, "r1"); assert.equal(draft.value, "Keep my typing"); assert.equal(draft.dirty, true);
  }
});
