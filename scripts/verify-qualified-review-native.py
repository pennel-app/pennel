#!/usr/bin/env python3
"""Exercise one real qualified Review job in an isolated, owned Capture Preview.

This is a paid functional smoke, not additional held-out quality evidence. It
requires an existing cumulative evaluation ledger and never retries inference.
"""

import argparse
from decimal import Decimal
import hashlib
import importlib.util
import json
from pathlib import Path
import platform
import subprocess
import sys
import time
import uuid


ROOT = Path(__file__).resolve().parents[1]
EVALUATION = ROOT / "evaluations/decision-models"
sys.path.insert(0, str(EVALUATION))
import finished_corpus_v2 as corpus  # noqa: E402
import finished_experiment as experiment  # noqa: E402

SPEC = importlib.util.spec_from_file_location("qualified_review_workspace", ROOT / "scripts/verify-live-workspace-native.py")
workspace = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(workspace)
capture = workspace.capture
TEAM = "B538Q2K76C"
MODEL = "perplexity/pplx-decider-v1.2-20261016"
NOTES = "# Owned private-note sentinel\n\nThis invented personal note must never enter Review inference.\n"
RESERVATION = Decimal("0.20")


OBSERVE = r"""
const original=window.fetch.bind(window),qa={commands:[]};
window.__PENNEL_NATIVE96__=qa;
window.fetch=(resource,options)=>{
 const url=new URL(typeof resource==='string'?resource:resource instanceof Request?resource.url:String(resource),location.href);
 if(url.protocol==='ipc:')qa.commands.push(decodeURIComponent(url.pathname.slice(1)));
 return original(resource,options);
};
return {observer_installed:true};
"""

SETUP = r"""
const account=__ACCOUNT__,info=await real('app_info'),s=info.settings;
Object.assign(s,{appearance:__APPEARANCE__,ui_language:__LOCALE__,history_retention_days:0,
 auto_detect:false,record_shortcut_enabled:false,calendar_enabled:false,cloud_sync:false,
 claude_access:false,llm:{provider_id:'',model:''},summary_automation:'manual',suggestion_automation:'unanswered',suggestion_language:'nl'});
s.onboarding.status='completed';
s.providers.push({id:account,kind:'open_router',name:'Owned Review smoke',base_url:'https://openrouter.ai/api/v1'});
s.decision={provider_id:account,protocol:'open_router_decisions',model:'unqualified-native-smoke',
 expected_model:null,upstream:'perplexity',max_input_bytes:16000,timeout_secs:30};
await real('save_settings',{settings:s});
return {isolated_settings_saved:true,automatic_analysis:false,text_model_selected:false};
"""

PRESET = r"""
const qa=window.__PENNEL_NATIVE96__,presets=await real('qualified_finished_suggestion_presets');
assert(presets.length===1,'Exactly one current-host adopted setup is required.');
const preset=presets[0];
assert(preset.identity_evidence==='provider_reported_version'&&preset.weights_independently_verified===false,
 'The adopted setup changed its identity policy.');
assert(preset.actual_model===__MODEL__&&preset.actual_provider==='Perplexity','The adopted model identity is different.');
assert(!qa.commands.includes('begin_insights_analysis'),'Setup unexpectedly started inference.');
document.querySelector('.library-settings').click();
await until(()=>document.querySelector('[data-settings-category="advanced"]'),'Settings did not open.');
document.querySelector('[data-settings-category="advanced"]').click();
await until(()=>document.querySelector('details[data-settings-detail="models"]'),'Models settings did not open.');
const models=document.querySelector('details[data-settings-detail="models"]');
if(!models.open)models.querySelector('summary').click();
await until(()=>document.querySelector('details[data-inference-experimental]'),'Experimental connections did not render.');
const experimental=document.querySelector('details[data-inference-experimental]');
if(!experimental.open)experimental.querySelector('summary').click();
const section=document.querySelector('[data-inference-capability="decision"]');
await until(()=>!section.querySelector('[data-tested-suggestion-apply]').disabled,'The adopted preset is unavailable.');
section.querySelector('[data-tested-suggestion-apply]').click();
await until(()=>section.querySelector('[data-inference-field="model"]').value===preset.configured.model,'The tested setup did not fill its draft.');
for(const field of ['model','expected_model','upstream','max_input_bytes','timeout_secs'])
 assert(String(section.querySelector('[data-inference-field="'+field+'"]').value)===String(preset.configured[field]),'Preset field differs: '+field);
assert((await real('app_info')).settings.decision.model==='unqualified-native-smoke','Applying the preset silently saved it.');
const save=[...section.querySelectorAll('button')].find(button=>['Save configuration','Configuratie bewaren'].includes(button.textContent.trim()));
assert(save&&!save.disabled,'Save configuration is unavailable.');save.click();
await until(async()=>(await real('app_info')).settings.decision?.expected_model===__MODEL__,'The tested configuration was not saved.');
const configured=(await real('app_info')).settings.decision;
assert(configured.provider_id===__ACCOUNT__,'Setup changed the selected account.');
assert(!qa.commands.some(command=>['verify_inference','inference_decide','begin_insights_analysis','set_key','download_model'].includes(command)),
 'Rendering or applying setup performed inference or changed credentials/models.');
qa.qualification=preset.qualification_id;
return {qualification_id:preset.qualification_id,configured,apply_was_draft_only:true,setup_inference_requests:0,
 weights_independently_verified:false,identity_evidence:preset.identity_evidence};
"""

OPEN = r"""
const id=__MEETING_ID__,qa=window.__PENNEL_NATIVE96__;
await real('plugin:event|emit_to',{target:{kind:'AnyLabel',label:'main'},event:'workspace-navigate',
 payload:{section:'meetings',tab:null,meeting_id:id,jump_ms:null}});
await until(()=>document.querySelector('.live-workspace')?.dataset.meetingId===id,'The owned finished meeting did not open.');
const button=document.querySelector('.live-workspace [data-live-tab="suggestions"]');
assert(button,'The Review tab is missing.');if(button.getAttribute('aria-selected')!=='true')button.click();
await until(()=>document.querySelector('.meeting-companion:not([hidden]) .insight-review'),'Review did not open.');
const root=document.querySelector('.insight-review');
await until(()=>root.querySelector('.suggestion-consent')?.dataset.mode!=='loading','Review metadata did not finish loading.');
const snapshot=await real('insights_snapshot',{id}),availability=await real('insights_availability',{id,languagePolicy:'nl'}),
 automatic=await real('automatic_insights_availability',{id,languagePolicy:'nl'}),detail=await real('meeting_detail',{id});
assert(availability.available&&availability.qualified&&availability.qualification_id===qa.qualification,'The real core did not admit manual Dutch Review.');
assert(/^[a-f0-9]{64}$/.test(availability.configuration_fingerprint),'The native availability has no exact configuration pin.');
assert(!automatic.available,'The manual-only record unexpectedly enabled automatic analysis.');
assert(detail.transcript.length===16&&detail.transcript.every(row=>row.utterance_id&&row.source_fingerprint&&!row.failed),
 'The real core did not expose all canonical synthetic sources.');
assert(detail.notes===__NOTES__,'Opening Review changed the private-note fixture.');
assert(!snapshot.items.length&&!snapshot.analysis_job,'A fresh profile already contains analysis.');
assert(!qa.commands.includes('begin_insights_analysis'),'Opening Review performed inference.');
const options=root.querySelector('.insight-analysis-details');if(!options.open)options.querySelector('summary').click();
await until(()=>{const find=root.querySelector('[data-suggestion-consent="find"]');return find&&!find.disabled&&find.getClientRects().length>0;},
 'The initial manual-consent Find did not become available.');
const checkbox=root.querySelector('[data-suggestion-consent-automatic]');
assert(checkbox.disabled&&!checkbox.checked,'The manual record offered automatic consent.');
assert((await real('app_info')).settings.suggestion_automation==='unanswered','Opening Review saved consent.');
qa.id=id;qa.beforeNotes=detail.notes;qa.expectedFingerprint=availability.configuration_fingerprint;
return {manual_available:true,automatic_available:false,automatic_reason:automatic.reason_code,
 opening_inference_requests:0,canonical_sources:detail.transcript.length,language_policy:availability.language_policy};
"""

START = r"""
const qa=window.__PENNEL_NATIVE96__,root=document.querySelector('.insight-review');
assert(!qa.commands.includes('begin_insights_analysis'),'Find was already invoked.');
const find=root.querySelector('[data-suggestion-consent="find"]');
assert(find&&!find.disabled&&find.getClientRects().length>0,'The visible initial-consent Find is unavailable.');
find.click();
await until(async()=>!!(await real('insights_snapshot',{id:qa.id})).analysis_job,'The genuine UI Find did not admit a job.');
const job=(await real('insights_snapshot',{id:qa.id})).analysis_job;
qa.jobId=job.job_id;
assert(qa.commands.filter(command=>command==='begin_insights_analysis').length===1,'Find did not produce exactly one native admission.');
assert(qa.commands.filter(command=>command==='set_meeting_suggestion_policy_checked').length===1,
 'Initial consent did not use exactly one checked native policy write.');
assert((await real('app_info')).settings.suggestion_automation==='manual','Initial consent enabled another policy.');
return {job_id:job.job_id,genuine_ui_find:true,initial_manual_consent:true,checked_policy_write:true};
"""

CHECK = r"""
const qa=window.__PENNEL_NATIVE96__,snapshot=await real('insights_snapshot',{id:qa.id}),detail=await real('meeting_detail',{id:qa.id});
assert(snapshot.items.length>0,'The actual provider returned no reviewable items.');
const encoder=new TextEncoder(),decoder=new TextDecoder('utf-8',{fatal:true});
const finalized=detail.transcript.filter(row=>!row.failed&&row.text.trim());
const semanticKey=item=>JSON.stringify([item.kind,item.commitment,item.text,item.owner??null,item.due??null]);
const expectedItems=new Set(__EXPECTED_ITEMS__.map(semanticKey)),seen=new Set();
for(const item of snapshot.items){
 const key=semanticKey(item);assert(expectedItems.has(key)&&!seen.has(key),'Native generation proposed an unsupported or duplicate development item.');seen.add(key);
 assert(item.source_state==='current'&&item.sources.length>=1&&item.sources.length<=3,'A generated source map is not current.');
 assert(item.owner===null&&item.due===null,'Passage review invented an owner or due date.');
 const identity=item.processing.identity;
 assert(identity.actual_model===__MODEL__&&identity.actual_provider==='Perplexity'&&identity.actual_revision===null,
  'Actual provider identity was lost, changed or fabricated.');
 assert(identity.protocol==='open_router_decisions'&&identity.requested_model==='perplexity/pplx-decider-v1.2'&&
  identity.requested_upstream==='perplexity'&&identity.endpoint==='https://openrouter.ai/api/alpha/decisions'&&identity.request_id,
  'Actual inference used an unexpected route.');
 assert(item.processing.qualification_id===qa.qualification&&item.processing.language_policy==='nl'&&
  item.processing.task==='meeting_suggestions_v1','A different qualification, language or task committed the items.');
 assert(item.extraction_scope==='passage'&&!item.related_item_ids.length&&!(item.prior_evidence??[]).length,
  'Passage review claimed unsupported extraction or prior links.');
 const centre=finalized.findIndex(row=>row.utterance_id===item.sources[0].utterance_id);
 assert(centre>=0,'The central source is missing.');
 const expected=[finalized[centre],...(centre>0?[finalized[centre-1]]:[]),...(centre+1<finalized.length?[finalized[centre+1]]:[])];
 assert(item.sources.length===expected.length,'The full central and adjacent source map differs.');
 for(let index=0;index<expected.length;index++){
  const source=item.sources[index],row=expected[index];
  assert(row.utterance_id===source.utterance_id&&row.source_fingerprint===source.source_fingerprint&&source.meeting_uid===snapshot.meeting_uid,
   'A durable source identity or its order differs.');
  assert(Number.isInteger(source.start_byte)&&Number.isInteger(source.end_byte)&&source.start_byte>=0&&
   source.start_byte<source.end_byte&&source.end_byte<=encoder.encode(row.text).length&&
   source.start_ms===row.start_ms&&source.end_ms===row.end_ms,'A source byte range or timestamp differs.');
  assert(decoder.decode(encoder.encode(row.text).slice(source.start_byte,source.end_byte))===source.quote,
   'A source quote differs from its exact finalized transcript bytes.');
 }
 assert(item.text===item.sources[0].quote,'The card changed its central passage.');
 assert(!item.text.includes('private-note sentinel'),'Personal notes entered generated suggestions.');
}
assert(detail.notes===qa.beforeNotes,'Analysis changed the personal notes.');
await until(()=>document.querySelectorAll('.insight-card').length===snapshot.items.length,'The generated cards did not render.');
qa.item=snapshot.items[0].id;qa.source=snapshot.items[0].sources[0];qa.count=snapshot.items.length;
return {items:snapshot.items,meeting_uid:snapshot.meeting_uid,notes_unchanged:true,
 exact_sources:true,actual_provider_generation:true,development_precision:1,development_recall:seen.size/expectedItems.size,
 remote_weights_independently_verified:false};
"""

ACCEPT = r"""
const qa=window.__PENNEL_NATIVE96__,card=document.querySelector('.insight-card[data-insight-id="'+qa.item+'"]');
card.querySelector('.insight-accept').click();
await until(async()=>(await real('insights_snapshot',{id:qa.id})).items.find(item=>item.id===qa.item)?.review_state==='accepted',
 'Acceptance did not persist.');
assert((await real('meeting_detail',{id:qa.id})).notes===qa.beforeNotes,'Acceptance wrote personal notes.');
const details=card.querySelector('.insight-card-details');if(details&&!details.open)details.querySelector('summary').click();
card.querySelector('.insight-source-open').click();
await until(()=>document.querySelector('.live-source-highlight'),'The exact source did not open.');
assert(getSelection().toString()===qa.source.quote,'The source action did not select the exact quote.');
document.querySelector('.live-source-back:not([hidden])').click();
await until(()=>document.querySelector('.meeting-companion:not([hidden]) .insight-review'),'Source return lost Review.');
const markdown=await real('insights_export',{id:qa.id});
assert(markdown.includes(qa.source.quote)&&!markdown.includes('private-note sentinel'),'Accepted export lost evidence or included personal notes.');
assert(!document.querySelector('.insight-export').disabled,'Accepted export is unavailable in the UI.');
assert(qa.commands.filter(command=>command==='begin_insights_analysis').length===1,'Review actions started another analysis.');
return {accepted_id:qa.item,accepted_quote:qa.source.quote,accepted_export:markdown,
 exact_source_navigation:true,notes_unchanged:true,clipboard_changed:false,ui_admissions:1};
"""

REOPEN = r"""
const qa=window.__PENNEL_NATIVE96__,id=__MEETING_ID__,expected=__EXPECTED__;
qa.qualification=expected.qualification_id;
await real('plugin:event|emit_to',{target:{kind:'AnyLabel',label:'main'},event:'workspace-navigate',
 payload:{section:'meetings',tab:null,meeting_id:id,jump_ms:null}});
await until(()=>document.querySelector('.live-workspace')?.dataset.meetingId===id,'The finished meeting did not reopen.');
const button=document.querySelector('.live-workspace [data-live-tab="suggestions"]');
if(button.getAttribute('aria-selected')!=='true')button.click();
await until(()=>document.querySelectorAll('.insight-card').length===expected.item_count,'Stored cards did not reopen.');
await pause(1500);
const snapshot=await real('insights_snapshot',{id}),detail=await real('meeting_detail',{id});
assert(snapshot.items.length===expected.item_count&&snapshot.items.find(item=>item.id===expected.accepted_id)?.review_state==='accepted',
 'Process restart lost generated items or acceptance.');
assert(detail.notes===__NOTES__,'Process restart changed personal notes.');
assert(!snapshot.analysis_job||snapshot.analysis_job.state==='completed'&&!snapshot.analysis_job.physical_running,
 'Process restart unexpectedly queued analysis.');
assert(!qa.commands.some(command=>['begin_insights_analysis','verify_inference','inference_decide'].includes(command)),
 'Opening stored Review after restart performed inference.');
return {process_restart:true,accepted_state_retained:true,stored_item_count:snapshot.items.length,
 reopen_inference_requests:0,analysis_job:snapshot.analysis_job??null,notes_unchanged:true};
"""


def expand(script, **values):
    for key, value in values.items():
        script = script.replace("__" + key.upper() + "__", json.dumps(value, ensure_ascii=False))
    return script


def write_fixture(profile):
    spec = corpus.experiment()
    fixture = next(item for item in spec["fixtures"] if item["id"] == spec["resource_fixture_id"])
    identifier = "2026-11-11 0900 Owned qualified Review"
    meeting = profile / "Meetings" / identifier
    meeting.mkdir(parents=True)
    uid = str(uuid.uuid4())
    (meeting / "meeting.md").write_text(
        "---\ntitle: Owned qualified Review\nstart: 2026-11-11T09:00:00+00:00\nduration_min: 2\n"
        f"template: general\npennel_id: {uid}\npennel_stt_language: nl\n---\n\n"
    )
    (meeting / "notes.md").write_text(NOTES)
    (meeting / "transcript.md").write_text("".join(
        f"[{row['start_ms']//60000:02}:{row['start_ms']//1000%60:02}] **Anderen:** {row['text']}\n"
        for row in fixture["utterances"]
    ))
    return identifier, {"id": fixture["id"], "turns": len(fixture["utterances"]), "quality_credit": False,
                        "expected_items": spec["gold_items"][fixture["id"]],
                        "sha256": hashlib.sha256(json.dumps(fixture, sort_keys=True, ensure_ascii=False).encode()).hexdigest()}


def poll_job(session, identifier, timeout=100):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        job = workspace.checkpoint(session, "return await real('insights_job',{jobId:" + json.dumps(identifier) + "});", "actual provider job status")
        if job["state"] in ("completed", "failed", "cancelled") and not job["physical_running"]:
            return job
        time.sleep(0.2)
    raise RuntimeError("The actual provider job did not terminate within its configured bound")


def billed_cost(job):
    if not job.get("billing_complete") or job.get("physical_running"):
        return None
    amount = job.get("reported_billed_cost_usd")
    if isinstance(amount, bool) or not isinstance(amount, (int, float)):
        return None
    value = Decimal(str(amount))
    return value if value.is_finite() and value >= 0 else None


def evaluation_credential(credential_fd=None):
    if credential_fd is not None:
        return experiment.credential(credential_fd).encode('utf-8')
    try:
        result = subprocess.run(["security", "find-generic-password", "-s", "dev.pennel.evaluation.openrouter",
                                 "-a", "OPENROUTER_API_KEY", "-w"], capture_output=True, check=False, timeout=15)
    except (OSError, subprocess.TimeoutExpired):
        raise ValueError("The dedicated evaluation credential could not be read within its bound") from None
    if result.returncode or not result.stdout.strip():
        raise ValueError("The dedicated evaluation credential is unavailable")
    return result.stdout.rstrip(b"\r\n")


def capture_credential(bridge, action, account, bundle, log, secret=None):
    arguments = [str(bridge), action, account]
    if action == "set":
        arguments.append(str(bundle))
    try:
        result = subprocess.run(arguments, input=secret, check=False, stdout=log, stderr=log, timeout=15)
    except (OSError, subprocess.TimeoutExpired):
        raise ValueError("The owned Capture credential operation did not finish within its bound") from None
    if result.returncode:
        raise ValueError("The owned Capture credential operation failed")


def capture_is_running(processes):
    for line in processes.splitlines():
        fields = line.strip().split(maxsplit=1)
        if len(fields) == 2 and "/Pennel Capture Preview.app/Contents/MacOS/" in fields[1]:
            return True
    return False


def validate_capture(binary, helper, credential_bridge):
    if platform.system() != "Darwin":
        raise ValueError("Owned native Review smoke requires macOS")
    capture.validate_binary(binary)
    for executable in (helper, credential_bridge):
        if not executable.is_file():
            raise ValueError("A required owned helper is missing")
    bundle = binary.parents[2]
    subprocess.run(["codesign", "--verify", "--deep", "--strict", str(bundle)], check=True, capture_output=True, timeout=15)
    signature = subprocess.run(["codesign", "-d", "--verbose=4", str(bundle)], check=True, capture_output=True, text=True, timeout=15)
    if f"TeamIdentifier={TEAM}" not in signature.stderr:
        raise ValueError("Capture Preview must use personal Apple Developer team B538Q2K76C")
    processes = subprocess.run(["ps", "-axo", "pid=,comm="], check=True, capture_output=True, text=True, timeout=15).stdout
    if capture_is_running(processes):
        raise ValueError("Another Capture Preview is already running; its session is preserved")
    return bundle


def main(arguments=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--execute", action="store_true", help="Explicitly execute one paid genuine UI Find")
    parser.add_argument("--binary", type=Path, default=capture.DEFAULT_BINARY)
    parser.add_argument("--window-helper", type=Path, required=True)
    parser.add_argument("--credential-bridge", type=Path, required=True)
    parser.add_argument("--credential-fd", type=int)
    parser.add_argument("--ledger", type=Path, default=experiment.DEFAULT_LEDGER)
    parser.add_argument("--output", type=Path, required=True, help="New owned output directory; existing results are never replaced")
    parser.add_argument("--locale", choices=("en", "nl"), default="en")
    parser.add_argument("--appearance", choices=("light", "dark"), default="dark")
    args = parser.parse_args(arguments)
    if not args.execute:
        parser.error("--execute is required; this command sends one paid synthetic Review job")
    binary, helper, bridge = (path.resolve() for path in (args.binary, args.window_helper, args.credential_bridge))
    bundle = validate_capture(binary, helper, bridge)
    output = args.output.resolve()
    output.mkdir(parents=True, mode=0o700, exist_ok=False)
    profile = output / "owned-capture-profile"
    profile.mkdir(mode=0o700)
    account = "review96-owned-" + uuid.uuid4().hex
    identifier, fixture = write_fixture(profile)
    receipt = {"complete": False, "scope": "genuine native functional smoke; no held-out quality credit",
               "capture_identifier": capture.CAPTURE_IDENTIFIER, "personal_team": TEAM,
               "actual_hardware_capture": False, "real_meeting_content": False, "automatic_retries": False,
               "fallbacks": False, "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
               "registry_sha256": hashlib.sha256((ROOT / "assets/qualifications/meeting-assistance.json").read_bytes()).hexdigest(),
               "fixture": fixture, "locale": args.locale, "appearance": args.appearance, "checks": {},
               "owned_capture_account": account, "capture_credential_creation_attempted": False,
               "capture_credential_created": False, "capture_credential_deleted": False}
    save = lambda: experiment.write_json(output / "receipt.json", receipt)
    save()
    with experiment.locked_ledger(args.ledger.resolve()) as ledger, (output / "app.log").open("w") as log:
        receipt["ledger_before"] = ledger.snapshot()
        if ledger.unresolved or ledger.pending or ledger.total_exposure + RESERVATION > ledger.limit:
            raise ValueError("The existing cumulative ledger cannot reserve this bounded smoke")
        session = capture.CaptureSession(binary, profile, log, helper)
        reservation = None
        try:
            session.launch()
            session.inspect("main")
            receipt["checks"]["initialize"] = workspace.checkpoint(session, expand(SETUP, account=account,
                appearance=args.appearance, locale=args.locale), "isolated setup without clearing browser storage")
            session.stop()
            save()
            # Initialize the owned profile before inserting its secret. Restarting also
            # clears absent-key cache entries before exercising the real configured UI.
            key = evaluation_credential(args.credential_fd)
            receipt["capture_credential_creation_attempted"] = True
            try:
                capture_credential(bridge, "set", account, bundle, log, key)
            finally:
                del key
            receipt["capture_credential_created"] = True
            save()
            session.launch("library")
            session.inspect("main")
            session.run("--capture-size", "main", "1400", "850")
            workspace.checkpoint(session, OBSERVE, "native IPC observer")
            receipt["checks"]["preset"] = workspace.checkpoint(session, expand(PRESET, model=MODEL, account=account), "adopted setup UI")
            receipt["checks"]["open"] = workspace.checkpoint(session, expand(OPEN, meeting_id=identifier, notes=NOTES), "manual Review without inference")
            images = output / args.locale / args.appearance
            receipt["before_image"] = workspace.image(session, images / "before-find.png")
            reservation = ledger.reserve(RESERVATION)
            save()
            started = time.monotonic()
            begun = workspace.checkpoint(session, START, "one genuine UI Find")
            receipt["job"] = poll_job(session, begun["job_id"])
            receipt["wall_elapsed_ms"] = (time.monotonic() - started) * 1000
            job = receipt["job"]
            cost = billed_cost(job)
            ledger.settle(reservation, cost, {"stage": "native_ui_smoke", "job_id": job["job_id"],
                "physical_request_count": job.get("completed_provider_requests"), "actual_provider_generation": True})
            reservation = None
            save()
            requests = job.get("completed_provider_requests")
            if (job["state"] != "completed" or isinstance(requests, bool) or not isinstance(requests, int)
                    or not 0 < requests <= fixture["turns"] or cost is None or ledger.unresolved):
                raise ValueError("The genuine native job did not complete its bounded, fully billed requests")
            receipt["checks"]["generated"] = workspace.checkpoint(session, expand(CHECK, model=MODEL,
                expected_items=fixture["expected_items"]), "genuine source-linked suggestions")
            receipt["after_image"] = workspace.image(session, images / "after-find.png")
            receipt["checks"]["accepted"] = workspace.checkpoint(session, ACCEPT, "accept source navigation and export")
            accepted = receipt["checks"]["accepted"]
            expected = {"qualification_id": receipt["checks"]["preset"]["qualification_id"],
                        "item_count": len(receipt["checks"]["generated"]["items"]), "accepted_id": accepted["accepted_id"]}
            session.stop()
            session.launch("library")
            session.inspect("main")
            workspace.checkpoint(session, OBSERVE, "restarted native IPC observer")
            receipt["checks"]["reopen"] = workspace.checkpoint(session, expand(REOPEN, meeting_id=identifier,
                expected=expected, notes=NOTES), "stored Review process restart")
            receipt["complete"] = True
        except Exception as error:
            receipt["failure_type"] = type(error).__name__
            # Error messages may include provider content; preserve only a bounded class.
            raise
        finally:
            try:
                if reservation is not None:
                    ledger.settle(reservation, None, {"stage": "native_ui_smoke", "reason": "job_billing_not_reconciled"})
            finally:
                session.stop()
                if receipt["capture_credential_creation_attempted"]:
                    try:
                        capture_credential(bridge, "delete", account, bundle, log)
                        receipt["capture_credential_deleted"] = True
                    except ValueError:
                        receipt["capture_credential_cleanup_incomplete"] = True
                        receipt["complete"] = False
                receipt["ledger_after"] = ledger.snapshot()
                save()
    print(json.dumps({"complete": receipt["complete"], "receipt": str(output / "receipt.json"),
                      "physical_requests": receipt.get("job", {}).get("completed_provider_requests"),
                      "known_billed_usd": receipt["ledger_after"]["known_billed_usd"]}))


if __name__ == "__main__":
    main()
