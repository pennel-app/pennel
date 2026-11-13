#!/usr/bin/env python3
"""Verify finished #57 review against real core IPC in an owned capture window."""

import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import platform
import subprocess
import tempfile
import time
import uuid


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "pennel_review_workspace", ROOT / "scripts/verify-live-workspace-native.py"
)
workspace = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(workspace)
capture = workspace.capture
TEAM = "B538Q2K76C"


DISCOVER = r"""
const meetings=await real('meetings',{query:''});
assert(meetings.length===2,'The dedicated capture data was not initialized.');
const info=meetings.find(value=>value.id.startsWith('2026-10-25 '));
assert(info,'The owned review meeting was not found.');
const detail=await real('meeting_detail',{id:info.id});
assert(detail.transcript.length===4,'The owned finalized transcript is incomplete.');
assert(detail.transcript.every(row=>row.utterance_id&&row.source_fingerprint&&!row.failed),
 'The real core did not return canonical finalized source identities.');
return {id:info.id,detail};
"""


OPEN_REVIEW = r"""
const id=__MEETING_ID__;
await real('plugin:event|emit_to',{target:{kind:'AnyLabel',label:'main'},event:'workspace-navigate',
 payload:{section:'meetings',tab:null,meeting_id:id,jump_ms:null}});
await until(()=>document.querySelector('.detail-tabs [data-tab="insights"]'),'The saved meeting did not open.');
document.querySelector('.detail-tabs [data-tab="insights"]').click();
await until(()=>document.querySelectorAll('.insight-card').length===4,'The real review store did not render.');
const snapshot=await real('insights_snapshot',{id});
assert(snapshot.status==='ready'&&snapshot.items.every(item=>item.source_state==='current'),
 'The seeded source anchors were not validated by the real core: '+JSON.stringify({status:snapshot.status,sources:snapshot.items.map(item=>item.source_state)}));
const qa={id,uid:snapshot.meeting_uid,root:document.querySelector('.insight-review'),
 items:snapshot.items.map(item=>item.id),beforeNotes:(await real('meeting_detail',{id})).notes,
 commands:[],revised:__REVISED__};
window.__PENNEL_NATIVE57__=qa;
const originalFetch=window.fetch.bind(window);
window.fetch=(resource,options)=>{
 const url=new URL(typeof resource==='string'?resource:resource instanceof Request?resource.url:String(resource),location.href);
 if(url.protocol==='ipc:')qa.commands.push(decodeURIComponent(url.pathname.slice(1)));
 return originalFetch(resource,options);
};
assert(qa.root.querySelector('.insight-find').disabled,'Opening review enabled unqualified analysis.');
const availability=[];
for(const languagePolicy of ['unknown','en','nl','mixed']){
 const value=await real('insights_availability',{id,languagePolicy});
 assert(!value.available&&!value.qualified,'The production registry unexpectedly qualified '+languagePolicy);
 availability.push({language_policy:languagePolicy,available:value.available,qualified:value.qualified,reason_code:value.reason_code});
}
const source=snapshot.items[0].sources[0];
assert(source.start_byte>source.quote.indexOf('Noor'),'The fixture did not exercise a UTF-8 prefix.');
const changed=qa.root.querySelector('[data-insight-id="'+qa.items[3]+'"]');
assert(!changed.querySelector('.insight-prior-statements').hidden&&!changed.querySelector('.insight-linkage').hidden,
 'Unresolved related evidence was presented as an established link.');
return {real_core_read:true,canonical_source_validation:true,source_metadata_status:(await real('meeting_detail',{id})).metadata_status,
 qualification_unavailable:availability,find_disabled:true,actual_capture:false,actual_provider_generation:false};
"""


ACCEPT_EDIT_DISMISS = r"""
const qa=window.__PENNEL_NATIVE57__,root=qa.root;
const store=()=>real('insights_snapshot',{id:qa.id});
const card=id=>root.querySelector('[data-insight-id="'+id+'"]');
const first=card(qa.items[0]),second=card(qa.items[1]);
first.querySelector('.insight-accept').click();
await until(async()=>(await store()).items.find(item=>item.id===qa.items[0]).review_state==='accepted',
 'Native acceptance was not durably stored.');
assert((await real('meeting_detail',{id:qa.id})).notes===qa.beforeNotes,'Acceptance silently wrote personal notes.');
first.querySelector('.insight-edit-action').click();
await until(()=>!first.querySelector('.insight-edit').hidden,'The claimed native editor did not open.');
const claimed=(await store()).items.find(item=>item.id===qa.items[0]);
assert(claimed.edit_claim,'The editor opened without a durable protection claim.');
const editor=first.querySelector('.insight-edit');editor.focus();
editor.dispatchEvent(new CompositionEvent('compositionstart',{bubbles:true}));
editor.value=qa.revised;editor.setSelectionRange(4,13,'backward');
editor.dispatchEvent(new InputEvent('input',{bubbles:true,isComposing:true,inputType:'insertCompositionText',data:'é 😀'}));
const before={text:editor.value,start:editor.selectionStart,end:editor.selectionEnd,direction:editor.selectionDirection};
await pause(850);await store();
assert(first.querySelector('.insight-edit')===editor&&document.activeElement===editor&&editor.value===before.text&&
 editor.selectionStart===before.start&&editor.selectionEnd===before.end&&editor.selectionDirection===before.direction,
 'Native review refresh changed the composing editor or selection.');
assert(first.querySelector('.insight-save-edit').disabled,'A composing edit could be committed.');
editor.dispatchEvent(new CompositionEvent('compositionend',{bubbles:true,data:'é 😀'}));
first.querySelector('.insight-save-edit').click();
await until(async()=>{const item=(await store()).items.find(item=>item.id===qa.items[0]);return item.edited_text===qa.revised&&!item.edit_claim;},
 'The native edited text or claim release was not durably stored.');
await until(()=>first.querySelector('.insight-edit').hidden,'The acknowledged native editor remained open.');
second.querySelector('.insight-dismiss').click();
await until(async()=>(await store()).items.find(item=>item.id===qa.items[1]).review_state==='dismissed',
 'Native dismissal was not durably stored.');
await until(()=>!second.isConnected,'The default native review list retained a dismissed card.');
root.querySelector('.insight-dismissed-toggle').click();
await until(()=>card(qa.items[1]),'The stored dismissed card could not be reopened.');
assert((await real('meeting_detail',{id:qa.id})).notes===qa.beforeNotes,'Review edits or dismissal wrote notes.');
return {accepted:true,edited_exactly:true,durable_edit_claim:true,composition_events_and_selection_preserved:true,
 actual_keyboard_ime_acceptance:false,dismissed:true,dismissed_reopen:true,notes_unchanged_before_explicit_insertion:true};
"""


SOURCE_RETURN = r"""
const qa=window.__PENNEL_NATIVE57__,root=qa.root;
const first=root.querySelector('[data-insight-id="'+qa.items[0]+'"]');
const source=(await real('insights_snapshot',{id:qa.id})).items.find(item=>item.id===qa.items[0]).sources[0];
const panel=document.querySelector('.meeting-panel');
root.scrollTop=70;panel.scrollTop=70;const scroll=root.scrollTop,panelScroll=panel.scrollTop;
const origin=first.querySelector('.insight-source-open');origin.focus({preventScroll:true});origin.click();
await until(()=>document.querySelector('.meeting-detail .source-highlight'),'The native exact source did not open.');
assert(getSelection().toString()===source.quote,'Native source navigation did not honor exact UTF-8 quote bounds.');
const row=document.querySelector('.meeting-detail .source-highlight');
const detail=await real('meeting_detail',{id:qa.id});
assert(detail.transcript[Number(row.dataset.transcriptIndex)]?.utterance_id===source.utterance_id,
 'Native source navigation selected a mutable person/timestamp identity.');
qa.source={quote_exact:true,durable_row_key:true,source_return_scroll:scroll,source_return_panel_scroll:panelScroll};
return {exact_quote:true,durable_row_key:true,utf8_prefix_exercised:true,review_scroll:[scroll,panelScroll]};
"""


RETURN_AND_INSERT = r"""
const qa=window.__PENNEL_NATIVE57__,root=qa.root;
document.querySelector('.insight-source-back').click();
await until(()=>root.isConnected,'Native source return lost the retained review component.');
await pause(100);
const first=root.querySelector('[data-insight-id="'+qa.items[0]+'"]');
assert(document.activeElement===first.querySelector('.insight-source-open'),'Source return did not restore its initiating control.');
assert(root.scrollTop===qa.source.source_return_scroll,'Source return lost the review scroll position.');
assert(document.querySelector('.meeting-panel').scrollTop===qa.source.source_return_panel_scroll,
 'Source return lost the parent reading position.');
first.querySelector('.insight-add-notes').click();
await until(async()=>{const value=await real('insights_snapshot',{id:qa.id});return value.insertion_receipts.some(receipt=>receipt.item_id===qa.items[0]&&receipt.state==='acknowledged');},
 'The ordinary native notes writer did not durably acknowledge insertion.');
const notes=(await real('meeting_detail',{id:qa.id})).notes;
assert(notes.startsWith(qa.beforeNotes)&&notes.includes(qa.revised),'Native insertion lost the personal note prefix or edited Markdown.');
assert(notes.split(qa.revised).length-1===1,'Native insertion duplicated accepted text.');
const receipt=(await real('insights_snapshot',{id:qa.id})).insertion_receipts.find(value=>value.item_id===qa.items[0]);
assert(receipt.markdown===qa.revised,'The notes receipt changed exact user-edited Markdown.');
qa.receipt=receipt.attempt_id;qa.insertedNotes=notes;
document.querySelector('.detail-tabs [data-tab="insights"]').click();
await until(()=>root.isConnected&&first.querySelector('.insight-add-notes').disabled,'Durable insertion did not disable repeated insertion.');
first.querySelector('.insight-add-notes').click();await pause(200);
assert((await real('meeting_detail',{id:qa.id})).notes===notes,'A repeated native click changed acknowledged notes.');
assert((await real('insights_snapshot',{id:qa.id})).insertion_receipts.length===1,'A repeated native click created another attempt.');
const markdown=await real('insights_export',{id:qa.id});
const snapshot=await real('insights_snapshot',{id:qa.id});
assert(markdown.startsWith(qa.revised)&&!markdown.includes(snapshot.items[1].text)&&!markdown.includes(qa.beforeNotes),
 'Native Markdown export included personal notes or dismissed text, or lost the accepted edit.');
const sourceExport=markdown.includes(snapshot.items[0].sources[0].quote)&&markdown.includes('[00:00]');
assert(!root.querySelector('.insight-export').disabled,'Accepted Markdown export is unavailable in native UI.');
assert(root.querySelector('.insight-find').disabled,'Native review or note insertion qualified analysis.');
const forbidden=['begin_insights_analysis','start_meeting','download_model','set_key','save_settings','start_voice_enrollment'];
for(const command of ['review_insight','claim_insight_edit','save_meeting_notes','insights_export'])
 assert(qa.commands.includes(command),'The native IPC observer did not observe '+command+'.');
assert(!qa.commands.some(command=>forbidden.includes(command)),'Review changed analysis, capture, models, credentials or settings.');
return {source_return_focus_and_scroll:true,ordinary_notes_writer:true,receipt_acknowledged:true,
 personal_notes_prefix_preserved:true,repeated_click_blocked:true,accepted_only_markdown_export:true,
 exact_user_edited_receipt_markdown:true,edited_export_body_verbatim:true,
 source_quote_and_timestamp_exported:sourceExport,find_still_disabled:true,
 ipc_observer_verified:true,observed_ipc_commands:[...new Set(qa.commands)].sort(),snapshot,notes,markdown};
"""


REOPEN = r"""
const id=__MEETING_ID__,expected=__EXPECTED__;
await real('plugin:event|emit_to',{target:{kind:'AnyLabel',label:'main'},event:'workspace-navigate',
 payload:{section:'meetings',tab:null,meeting_id:id,jump_ms:null}});
await until(()=>document.querySelector('.detail-tabs [data-tab="insights"]'),'The meeting did not reopen.');
document.querySelector('.detail-tabs [data-tab="insights"]').click();
await until(()=>document.querySelector('.insight-review .insight-card'),'Review did not reopen after process restart.');
const snapshot=await real('insights_snapshot',{id}),detail=await real('meeting_detail',{id});
const accepted=snapshot.items.find(item=>item.id===expected.accepted),dismissed=snapshot.items.find(item=>item.id===expected.dismissed);
assert(accepted.review_state==='accepted'&&accepted.edited_text===expected.revised&&!accepted.edit_claim,'Restart lost the accepted edit.');
assert(dismissed.review_state==='dismissed','Restart lost dismissal.');
assert(snapshot.insertion_receipts.length===1&&snapshot.insertion_receipts[0].attempt_id===expected.attempt&&snapshot.insertion_receipts[0].state==='acknowledged',
 'Restart lost the insertion acknowledgement.');
assert(detail.notes===expected.notes,'Restart changed the acknowledged personal notes.');
const root=document.querySelector('.insight-review');
assert(root.querySelector('.insight-find').disabled,'Restart silently enabled analysis.');
assert(root.querySelector('[data-insight-id="'+expected.accepted+'"] .insight-add-notes').disabled,'Restart permitted duplicate insertion.');
return {process_restart:true,accepted_edit_retained:true,dismissed_retained:true,acknowledged_receipt_retained:true,
 notes_retained:true,duplicate_insertion_still_blocked:true,find_disabled:true};
"""


def write_owned_meetings(data, locale):
    capture.write_fixtures(data, locale)
    identifier = capture.fixture_meetings(locale)[0][0]
    folder = data / "Meetings" / identifier
    meeting = folder / "meeting.md"
    meeting.write_text(meeting.read_text().replace("---\n", "---\npennel_id: " + str(uuid.uuid4()) + "\n", 1))
    if locale == "en":
        texts = ["Café 😀. Noor will share the agenda by Friday.",
                 "We decided to keep the Friday release.",
                 "Is the client demo confirmed?",
                 "Noor will not share the agenda; Sam will take over."]
        revised = "- [ ] Reviewed action kept exactly: café 😀.\n  Personal accepted edit."
    else:
        texts = ["Café 😀. Noor stuurt de agenda tegen vrijdag.",
                 "We besluiten de release van vrijdag te behouden.",
                 "Is de klantdemo bevestigd?",
                 "Noor stuurt de agenda niet; Sam neemt het over."]
        revised = "- [ ] Nagekeken actie exact bewaard: café 😀.\n  Persoonlijke aanvaarde wijziging."
    # The readable storage format keeps its role tokens independent of interface language.
    transcript = "".join(f"[00:{index * 10:02d}] **{'Ik' if index % 2 else 'Anderen'}:** {text}\n"
                         for index, text in enumerate(texts))
    (folder / "transcript.md").write_text(transcript)
    return identifier, revised


def seed_insights(data, identifier, detail, locale):
    folder = data / "Meetings" / identifier
    if Path(detail["dir"]).resolve() != folder.resolve():
        raise RuntimeError("The real meeting detail escaped the dedicated capture directory")
    rows = detail["transcript"]
    digest = hashlib.sha256((folder / "transcript.md").read_bytes()).hexdigest()
    cutoff = max(row["end_ms"] for row in rows)
    sources = []
    for index, row in enumerate(rows):
        uuid.UUID(row["utterance_id"])
        if len(row["source_fingerprint"]) != 64:
            raise RuntimeError("The native source fingerprint is malformed")
        quote = row["text"].split(". ", 1)[1] if index == 0 else row["text"]
        start = len(row["text"].split(quote, 1)[0].encode("utf-8"))
        sources.append({"meeting_uid": detail["meeting_uid"], "utterance_id": row["utterance_id"],
                        "source_fingerprint": row["source_fingerprint"], "quote": quote,
                        "start_byte": start, "end_byte": start + len(quote.encode("utf-8")),
                        "start_ms": row["start_ms"], "end_ms": row["end_ms"]})
    ids = [str(uuid.uuid4()) for _ in rows]
    items = []
    for index, (kind, commitment) in enumerate((("action", "confirmed"), ("decision", "confirmed"),
                                                 ("open_question", "unresolved"), ("changed_commitment", "changed"))):
        quote = sources[index]["quote"]
        primary = [sources[index]["utterance_id"], sources[index]["source_fingerprint"], sources[index]["start_byte"]]
        dedup = json.dumps([kind, " ".join(quote.split()).lower(), primary], ensure_ascii=False, separators=(",", ":"))
        due = "by Friday" if locale == "en" else "tegen vrijdag"
        due_start = len(quote.split(due, 1)[0].encode("utf-8"))
        items.append({"id": ids[index], "dedup_key": hashlib.sha256(dedup.encode()).hexdigest(), "revision": 1,
                      "kind": kind, "extraction_scope": "exact_span", "commitment": commitment, "text": quote,
                      "owner": {"text": "Noor", "source_index": 0, "start_byte": 0, "end_byte": 4} if index == 0 else None,
                      "due": {"text": due, "source_index": 0, "start_byte": due_start,
                              "end_byte": due_start + len(due.encode())} if index == 0 else None,
                      "sources": [sources[index]], "evidence_cutoff_ms": cutoff, "transcript_revision": detail["revision"],
                      "transcript_digest": digest,
                      "processing": {"identity": {"protocol": "local_decider", "endpoint": "http://127.0.0.1:9/owned-seed-only",
                                                   "requested_model": "Owned fixture — no inference", "requested_upstream": None,
                                                   "actual_model": None, "actual_revision": None, "actual_provider": None, "request_id": None},
                                     "qualification_id": "owned-seeded-review-only-not-production-qualification",
                                     "language_policy": locale, "task": "meeting_suggestions_v1"},
                      "related_item_ids": [], "prior_evidence": [{"kind": "action", "source": sources[0], "target_item_id": None}] if index == 3 else [],
                      "review_state": "proposed", "edited_text": None, "edit_claim": None, "source_state": "current",
                      "source_history": [], "source_reassociated": False})
    document = {"schema_version": 1, "meeting_uid": detail["meeting_uid"], "revision": 1,
                "items": items, "insertion_receipts": []}
    destination = folder / "insights.json"
    if destination.exists():
        raise RuntimeError("Refusing to overwrite existing review evidence")
    destination.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n")
    return document


def compiled_helper(options, output):
    if options.window_helper:
        helper = options.window_helper.resolve()
        if not helper.is_file():
            raise RuntimeError("The supplied owned-window helper is missing")
        return helper
    source = output / "owned-window.swift"
    helper = output / "owned-window"
    source.write_text(capture.WINDOW_CHECK)
    arguments = ["swiftc", "-O", str(source), "-o", str(helper)]
    guard = Path.home() / ".local/bin/worktree-storage"
    if guard.is_file():
        arguments = [str(guard), "run", "--", *arguments]
    with (output / "window-helper-build.log").open("w") as log:
        subprocess.run(arguments,
                       check=True, stdout=log, stderr=subprocess.STDOUT, timeout=240)
    return helper


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=capture.DEFAULT_BINARY)
    parser.add_argument("--window-helper", type=Path, help="Reuse a compiled original WINDOW_CHECK helper")
    parser.add_argument("--output", type=Path, default=ROOT / "build/issue57/native-reviewed-insights")
    parser.add_argument("--locale", choices=("both", "en", "nl"), default="both")
    parser.add_argument("--appearance", choices=("both", "light", "dark"), default="both")
    options = parser.parse_args()
    if platform.system() != "Darwin":
        raise SystemExit("Owned native review verification requires macOS")
    binary = options.binary.resolve()
    capture.validate_binary(binary)
    bundle = binary.parents[2]
    subprocess.run(["codesign", "--verify", "--deep", "--strict", str(bundle)], check=True)
    signature = subprocess.run(["codesign", "-d", "--verbose=4", str(bundle)], capture_output=True, text=True, check=True)
    if "TeamIdentifier=" + TEAM not in signature.stderr:
        raise SystemExit("Native verification requires personal Apple Developer team " + TEAM)
    output = options.output.resolve()
    if (output / "manifest.json").exists():
        raise SystemExit("Preserve the existing evidence; select a new --output directory")
    output.mkdir(parents=True, exist_ok=True)
    manifest = {"complete": False, "native_tauri_app": True, "actual_core_ipc": True, "ipc_mocks": False,
                "owned_seeded_review_data": True, "actual_hardware_capture": False, "actual_provider_generation": False,
                "model_quality_acceptance": False, "actual_keyboard_ime_acceptance": False, "personal_team": TEAM,
                "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "os_version": platform.mac_ver()[0],
                "captured_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), "checks": [], "screenshots": []}
    save = lambda: (output / "manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n")
    save()
    helper = compiled_helper(options, output)
    locales = ("en", "nl") if options.locale == "both" else (options.locale,)
    appearances = ("light", "dark") if options.appearance == "both" else (options.appearance,)
    with tempfile.TemporaryDirectory(prefix="pennel-native57-") as temporary, (output / "app.log").open("w") as log:
        work = Path(temporary)
        backdrop = capture.NativeBackdrop(helper, work / "backdrop.json", log)
        backdrop.start()
        try:
            for locale in locales:
                for appearance in appearances:
                    group = {"locale": locale, "appearance": appearance, "checks": {}}
                    manifest["checks"].append(group)
                    data = work / (locale + "-" + appearance)
                    data.mkdir()
                    session = capture.CaptureSession(binary, data, log, helper, backdrop)
                    try:
                        session.initialize(locale, appearance)
                        settings_digest = hashlib.sha256((data / "settings.json").read_bytes()).hexdigest()
                        identifier, revised = write_owned_meetings(data, locale)
                        session.launch("library")
                        session.inspect("main")
                        discovered = workspace.checkpoint(session, DISCOVER, "real finalized source discovery")
                        if discovered["id"] != identifier:
                            raise RuntimeError("The native meeting ID differs from the owned fixture")
                        session.stop()
                        seeded = seed_insights(data, identifier, discovered["detail"], locale)
                        evidence_dir = output / locale / appearance
                        evidence_dir.mkdir(parents=True)
                        (evidence_dir / "canonical-meeting-detail.json").write_text(json.dumps(discovered, ensure_ascii=False, indent=2) + "\n")
                        (evidence_dir / "seeded-insights.json").write_text(json.dumps(seeded, ensure_ascii=False, indent=2) + "\n")
                        session.launch("library")
                        session.inspect("main")
                        session.run("--capture-size", "main", "1400", "850")
                        body = OPEN_REVIEW.replace("__MEETING_ID__", json.dumps(identifier)).replace("__REVISED__", json.dumps(revised))
                        group["checks"]["open"] = workspace.checkpoint(session, body, "native unqualified review")
                        save()
                        for width, height, name in ((1400, 850, "review-wide"), (760, 520, "review-minimum")):
                            session.run("--capture-size", "main", str(width), str(height))
                            time.sleep(0.35)
                            relative = Path(locale) / appearance / (name + ".png")
                            manifest["screenshots"].append({"file": str(relative), **workspace.image(session, output / relative)})
                            if name == "review-minimum":
                                group["checks"]["minimum_scroll"] = workspace.checkpoint(session,
                                    "const card=document.querySelector('.insight-card');card.scrollIntoView({block:'start'});await pause(80);"
                                    "const panel=document.querySelector('.meeting-panel'),bounds=card.getBoundingClientRect();"
                                    "assert(panel.scrollTop>0&&bounds.top<innerHeight&&bounds.bottom>0,'Minimum-size review cards are not reachable by scrolling.');"
                                    "return {cards_reachable:true,reading_scroll:panel.scrollTop};", "native minimum-size review scroll")
                                relative = Path(locale) / appearance / "review-minimum-card.png"
                                manifest["screenshots"].append({"file": str(relative), **workspace.image(session, output / relative)})
                                workspace.checkpoint(session, "document.querySelector('.meeting-panel').scrollTop=0;return {reset:true};", "native review scroll reset")
                            save()
                        session.run("--capture-size", "main", "1400", "850")
                        group["checks"]["review"] = workspace.checkpoint(session, ACCEPT_EDIT_DISMISS, "native accept/edit/dismiss")
                        group["checks"]["source"] = workspace.checkpoint(session, SOURCE_RETURN, "native exact source")
                        relative = Path(locale) / appearance / "source-exact.png"
                        manifest["screenshots"].append({"file": str(relative), **workspace.image(session, output / relative)})
                        inserted = workspace.checkpoint(session, RETURN_AND_INSERT, "native ordinary note insertion/export")
                        evidence = output / locale / appearance / "real-backend.json"
                        evidence.write_text(json.dumps(inserted, ensure_ascii=False, indent=2) + "\n")
                        markdown = output / locale / appearance / "accepted.md"
                        markdown.write_text(inserted.pop("markdown"))
                        snapshot = inserted.pop("snapshot")
                        notes = inserted.pop("notes")
                        group["checks"]["insert_export"] = inserted
                        save()
                        if not inserted["source_quote_and_timestamp_exported"]:
                            raise RuntimeError("Native export lost the accepted edited item's source quote/timestamp")
                        session.stop()
                        persisted = json.loads((data / "Meetings" / identifier / "insights.json").read_text())
                        if persisted["revision"] != snapshot["revision"] or (data / "Meetings" / identifier / "notes.md").read_text() != notes:
                            raise RuntimeError("The acknowledged native result differs from durable disk state")
                        group["checks"]["disk_acknowledgement"] = True
                        session.launch("library")
                        session.inspect("main")
                        session.run("--capture-size", "main", "1400", "850")
                        expected = {"accepted": seeded["items"][0]["id"], "dismissed": seeded["items"][1]["id"],
                                    "attempt": snapshot["insertion_receipts"][0]["attempt_id"], "notes": notes, "revised": revised}
                        body = REOPEN.replace("__MEETING_ID__", json.dumps(identifier)).replace("__EXPECTED__", json.dumps(expected))
                        group["checks"]["restart"] = workspace.checkpoint(session, body, "native persisted review restart")
                        if hashlib.sha256((data / "settings.json").read_bytes()).hexdigest() != settings_digest:
                            raise RuntimeError("Review changed isolated provider/model or application settings")
                        group["checks"]["settings_unchanged"] = True
                        relative = Path(locale) / appearance / "review-reopened.png"
                        manifest["screenshots"].append({"file": str(relative), **workspace.image(session, output / relative)})
                        save()
                        print(json.dumps({"locale": locale, "appearance": appearance, "complete": True}), flush=True)
                    except Exception as error:
                        group["error"] = str(error)
                        save()
                        raise
                    finally:
                        session.stop()
        finally:
            backdrop.stop()
    manifest["complete"] = True
    if hashlib.sha256(binary.read_bytes()).hexdigest() != manifest["binary_sha256"]:
        manifest["complete"] = False
        save()
        raise RuntimeError("The native capture binary changed during verification")
    subprocess.run(["codesign", "--verify", "--deep", "--strict", str(bundle)], check=True)
    save()
    print(str(output / "manifest.json"), flush=True)


if __name__ == "__main__":
    main()
