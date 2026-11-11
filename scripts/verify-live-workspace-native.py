#!/usr/bin/env python3
"""Exercise #95 in an owned Tauri window without opening any audio capture device."""

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
SPEC = importlib.util.spec_from_file_location("pennel_owned_capture", ROOT / "scripts/capture-desktop-windows.py")
capture = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(capture)


REPORT = r"""
const real = window.__PENNEL_NATIVE95__?.real ?? window.__TAURI_INTERNALS__.invoke.bind(window.__TAURI_INTERNALS__);
const assert = (value, message) => { if (!value) throw new Error(message); };
const pause = (ms = 30) => new Promise(resolve => setTimeout(resolve, ms));
const until = async (check, message) => { for(let index=0;index<200;index++){if(await check())return;await pause();}throw new Error(message); };
const edit = (input, text) => { if(input.applyUserChange){input.applyUserChange(text);return;}input.value=text;input.dispatchEvent(new InputEvent('input',{bubbles:true,inputType:'insertText',data:text})); };
const replaceSelection = (note,text) => {const start=note.selectionStart,end=note.selectionEnd;note.applyUserChange(note.value.slice(0,start)+text+note.value.slice(end),{start,end:start+text.length});};
const tab = (root, pane) => {if(pane==='notes'||pane==='summary'){if(root.classList.contains('live-overlay'))root.querySelector('.meeting-companion-close').click();const doc=root.querySelector('.meeting-document'),section=root.querySelector('[data-live-pane="'+pane+'"]');doc.scrollTop+=section.getBoundingClientRect().top-doc.getBoundingClientRect().top;return;}const button=root.querySelector('[data-live-tab="'+pane+'"]');if(button.getAttribute('aria-selected')!=='true')button.click();};
let result = null, failure = '';
try { result = await (async()=>{ __BODY__ })(); }
catch(error) { failure = String(error).slice(0,2048); }
const environment = await real('desktop_environment');
await real('capture_snapshot',{snapshot:{request_id:__REQUEST__,environment,
 document_language:document.documentElement.lang,appearance:document.documentElement.dataset.appearance||'system',
 ready:true,error:failure,selected_tab:JSON.stringify(result),viewport:[innerWidth,innerHeight],scroll_width:document.documentElement.scrollWidth}});
"""


REAL_MEETING = r"""
await until(()=>document.querySelector('.live-notes .cm-content'),'The saved native meeting did not open.');
const note=document.querySelector('.live-notes');
const text='Owned native saved note. Synthetic data only.';
edit(note,text);
document.querySelector('.library-settings').click();
await until(()=>document.querySelector('.settings-back'),'Real Settings did not open.');
document.querySelector('.settings-back').click();
await until(()=>document.querySelector('.live-notes')?.value===text,'Real Settings return lost the saved note.');
return {real_backend_storage:true,real_settings_return:true,synthetic_data_only:true};
"""


INSTALL_LIVE = r"""
const infos=await real('meetings',{query:''}); assert(infos.length,'No owned saved meeting fixture.');
const saved=await real('meeting_detail',{id:infos[0].id});
const originalInfo=await real('app_info');
const qa={real,id:infos[0].id,uid:'native95-owned-uid',session:'native95-owned-session',title:'Synthetic live meeting',
 notes:saved.notes,revision:1,recording:true,failSaves:false,requests:new Map(),stopCount:0,
 transcript:[{start_ms:0,end_ms:8000,speaker:'others',text:'Synthetic planning is confirmed for Monday.',failed:false,voice:1},
 {start_ms:8000,end_ms:14000,speaker:'me',text:'I will confirm the next synthetic appointment.',failed:false,voice:null}]};
window.__PENNEL_NATIVE95__=qa;
const originalInvoke=real, originalFetch=window.fetch.bind(window);
qa.bypass=0;qa.fetchIntercepts=0;
qa.real=(command,args={})=>{qa.bypass++;try{return originalInvoke(command,args);}finally{qa.bypass--;}};
const hash=async text=>Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',new TextEncoder().encode(text))),byte=>byte.toString(16).padStart(2,'0')).join('');
const status=(content=true)=>({recording:qa.recording,phase:qa.recording?'recording':'idle',busy:qa.recording,error:null,
 session_id:qa.recording?qa.session:null,meeting_uid:qa.recording?qa.uid:null,id:qa.recording?qa.id:null,title:qa.title,
 elapsed_secs:754,system_audio:true,heard_system_audio:true,echo_cancellation_active:false,audio_warning:null,
 persistence_error:null,transcription_warning:null,terminal_error:null,can_retry_persistence:false,pending:0,transcript_revision:qa.revision,
 recognition:{provider_kind:'local',provider_name:'Synthetic capture IPC',model:'fixture-only',language:'en'},
 ...(content?{notes:qa.notes,transcript:structuredClone(qa.transcript)}:{})});
qa.complete=async requestId=>{const snapshot=qa.requests.get(requestId);if(!snapshot)return;await real('plugin:event|emit',{event:'live-question-result',payload:{snapshot,
 answer:{text:'Native synthetic answer [1]',citations:[{number:1,passage:snapshot.anchors[0].passage}]},error:null}});};
const syntheticInvoke=async(command,args={})=>{
 if(command==='start_meeting')throw new Error('This QA run must never start actual audio capture.');
 if(command==='app_info'){
  const info=structuredClone(originalInfo);
  info.settings.providers.push({id:'native95-provider',kind:'custom',name:'Synthetic provider IPC',base_url:'https://native-fixture.invalid/v1'});
  info.settings.llm={provider_id:'native95-provider',model:'synthetic-only'};
  info.keys['native95-provider']=true;
  const kind=info.kinds.find(kind=>kind.kind==='custom');if(kind)kind.availability={status:'available',reason:null};
  return info;
 }
 if(command==='meeting_status')return status();
 if(command==='meeting_runtime_status')return status(false);
 if(command==='set_meeting_title'){qa.title=String(args.title??'');return null;}
 if(command==='set_meeting_notes'||command==='save_meeting_notes'){
  if(qa.failSaves)throw new Error('Owned synthetic note write failure.');
  qa.notes=String(args.text??'');
  if(command==='save_meeting_notes')return qa.real(command,args);
  return null;
 }
 if(command==='begin_live_question'){
  assert(args.expectedNotesText===qa.notes,'Unacknowledged notes entered the synthetic question.');
  const anchors=[];const occurrences=new Map();
  for(const segment of qa.transcript){const base=segment.speaker+':'+segment.start_ms;const occurrence=occurrences.get(base)||0;occurrences.set(base,occurrence+1);
   const text=(segment.speaker==='me'?'Ik: ':'Anderen: ')+segment.text;
   anchors.push({passage:{meeting_id:qa.uid,title:qa.title,start_iso:infos[0].start_iso,kind:'transcript',start_ms:segment.start_ms,text,snippet:text},
    segment_keys:[base+':'+occurrence],segment_fingerprints:[await hash(JSON.stringify([segment.start_ms,segment.speaker,segment.text.trim().replace(/\p{White_Space}+/gu,' '),segment.failed,segment.speaker==='others'?segment.voice??null:null]))],
    notes_start:null,notes_end:null,source_text:segment.text});}
  const snapshot={request_id:args.requestId,meeting_uid:qa.uid,session_id:qa.session,transcript_revision:qa.revision,notes_revision:await hash(qa.notes),
   included_through_ms:Math.max(0,...qa.transcript.map(segment=>segment.end_ms)),pending_transcription:0,anchors};
  qa.requests.set(args.requestId,snapshot);return snapshot;
 }
 if(command==='cancel_live_question'){qa.requests.delete(args.requestId);return null;}
 if(command==='stop_meeting'){
  qa.recording=false;qa.stopCount++;
  await real('plugin:event|emit',{event:'meeting-finished',payload:{id:qa.id,meeting_uid:qa.uid,session_id:qa.session,error:null,
   flush_warning:qa.failSaves?'Owned synthetic note write failure.':null}});
  return null;
 }
 return qa.real(command,args);
};
const intercepted=new Set(['start_meeting','app_info','meeting_status','meeting_runtime_status','set_meeting_title','set_meeting_notes','save_meeting_notes','begin_live_question','cancel_live_question','stop_meeting']);
window.fetch=async(resource,options)=>{
 const url=typeof resource==='string'?resource:resource instanceof Request?resource.url:String(resource);
 const parsed=new URL(url,location.href),command=decodeURIComponent(parsed.pathname.slice(1));
 if(!qa.bypass&&parsed.protocol==='ipc:'&&intercepted.has(command)){
  qa.fetchIntercepts++;let value,success=true;
  try{value=await syntheticInvoke(command,JSON.parse(options?.body||'{}'));}catch(error){value=error instanceof Error?error.message:String(error);success=false;}
  return new Response(JSON.stringify(value),{status:200,headers:{'Content-Type':'application/json','Tauri-Response':success?'ok':'error'}});
 }
 return originalFetch(resource,options);
};
await real('plugin:event|emit',{event:'settings-changed',payload:null});
await pause(200);
assert((await window.__TAURI_INTERNALS__.invoke('meeting_status')).session_id===qa.session,'The owned synthetic IPC fetch transport did not attach.');
await real('plugin:event|emit_to',{target:{kind:'AnyLabel',label:'main'},event:'workspace-navigate',payload:{section:'recording',tab:null,meeting_id:qa.id,jump_ms:null}});
await until(()=>document.querySelector('.live-workspace')?.dataset.sessionId===qa.session,'The built native workspace did not open.');
qa.root=document.querySelector('.live-workspace');qa.editor=qa.root.querySelector('.live-notes');
return {native_tauri_window:true,capture_ipc:'synthetic',provider_ipc:'synthetic',actual_audio_capture:false,actual_provider_generation:false};
"""


LIVE_CONCURRENCY = r"""
const qa=window.__PENNEL_NATIVE95__,root=qa.root,note=qa.editor;
edit(note,('# Synthetic native notes\n\nConcurrent input stays local.\n').repeat(16));
tab(root,'questions');edit(root.querySelector('.live-question-input'),'What was confirmed in the synthetic meeting?');
root.querySelector('.live-question-submit').click();
await until(()=>qa.requests.size===1,'The synthetic native question did not start.');
const request=[...qa.requests.values()][0];tab(root,'notes');note.focus();note.setSelectionRange(5,15,'backward');
note.contentDOM.dispatchEvent(new CompositionEvent('compositionstart',{bubbles:true,data:''}));
replaceSelection(note,'samengesteld');
const before={value:note.value,start:note.selectionStart,end:note.selectionEnd,direction:note.selectionDirection,scroll:note.scrollTop};
qa.transcript.push({start_ms:14000,end_ms:21000,speaker:'others',text:'Later synthetic speech is outside the earlier question.',failed:false,voice:1});qa.revision++;
await qa.complete(request.request_id);
await until(()=>root.textContent.includes('Native synthetic answer'),'The native synthetic answer did not render.');
await pause(750);
assert(root.querySelector('.live-notes')===note&&document.activeElement===note.contentDOM&&note.value===before.value&&note.selectionStart===before.start&&note.selectionEnd===before.end&&note.selectionDirection===before.direction&&note.scrollTop===before.scroll,'Native updates changed editor/composition state.');
assert(request.included_through_ms===14000,'The synthetic answer included later transcript.');
note.contentDOM.dispatchEvent(new CompositionEvent('compositionend',{bubbles:true,data:'samengesteld'}));
await until(()=>qa.notes===note.value,'Completed native composition was not acknowledged.');
const selection={start:note.selectionStart,end:note.selectionEnd,direction:note.selectionDirection,scroll:note.scrollTop};
tab(root,'questions');root.querySelector('.citation').click();
await until(()=>root.querySelector('.live-source-highlight'),'The native answer source did not open.');
root.querySelector('.live-source-back').click();
assert(note.selectionStart===selection.start&&note.selectionEnd===selection.end&&note.selectionDirection===selection.direction&&note.scrollTop===selection.scroll,'Native source return lost the editor selection.');
return {same_editor:true,composition_events_preserved:true,selection_preserved:true,frozen_sources:true,
 cited_source_and_return:true,actual_keyboard_ime_acceptance:false,capture_and_provider_ipc:'synthetic'};
"""


LIVE_SETTINGS = r"""
const qa=window.__PENNEL_NATIVE95__,note=qa.editor;
edit(note,'Owned native draft across Settings. Synthetic data only.');note.setSelectionRange(3,14,'backward');
document.querySelector('.library-settings').click();
await until(()=>document.querySelector('.settings-back'),'Native Settings did not open during the synthetic recording.');
assert([...document.querySelectorAll('[data-capture-stop]')].some(button=>!button.hidden&&!button.disabled),'Settings hid Stop.');
document.querySelector('.settings-back').click();
await until(()=>document.querySelector('.live-workspace')===qa.root,'Native Settings did not return to the live workspace.');
assert(document.querySelector('.live-notes')===note&&note.value==='Owned native draft across Settings. Synthetic data only.'&&note.selectionStart===3&&note.selectionEnd===14,'Native Settings lost editor, draft or selection.');
await until(()=>qa.notes===note.value,'The native live draft was not acknowledged before hide.');
return {real_native_settings:true,same_editor:true,selection_preserved:true,capture_control_reachable:true};
"""


LIVE_REOPEN = r"""
const qa=window.__PENNEL_NATIVE95__;
await until(()=>document.querySelector('.live-workspace')===qa.root,'Reopen did not retain the native live workspace.');
assert(document.querySelector('.live-notes')===qa.editor&&qa.editor.value==='Owned native draft across Settings. Synthetic data only.','Native hide/reopen lost the editor or draft.');
return {same_live_editor:true,draft_retained:true};
"""


LIVE_STOP_FAILURE = r"""
const qa=window.__PENNEL_NATIVE95__,note=qa.editor;
const count=qa.requests.size;tab(qa.root,'questions');edit(qa.root.querySelector('.live-question-input'),'Finish this synthetic answer after Stop.');
qa.root.querySelector('.live-question-submit').click();
await until(()=>qa.requests.size>count,'The native question before Stop did not start.');
const pending=[...qa.requests.values()].at(-1);
qa.failSaves=true;tab(qa.root,'notes');edit(note,'Owned native unsaved draft survives Stop. Synthetic data only.');
qa.root.querySelector('.live-save').click();
await until(()=>!qa.root.querySelector('.live-save-error').hidden,'The synthetic failed note write was not observed.');
note.focus();note.setSelectionRange(6,12,'backward');
note.contentDOM.dispatchEvent(new CompositionEvent('compositionstart',{bubbles:true,data:''}));
replaceSelection(note,'samengesteld');
const before={value:note.value,start:note.selectionStart,end:note.selectionEnd,direction:note.selectionDirection,scroll:note.scrollTop};
const started=performance.now();await window.__TAURI_INTERNALS__.invoke('stop_meeting');
await until(()=>!qa.recording,'A failed synthetic note write prevented native Stop dispatch.');
const stopped=performance.now()-started;
await pause(500);
assert(document.querySelector('.live-notes')===note&&document.activeElement===note.contentDOM&&note.value===before.value&&note.selectionStart===before.start&&note.selectionEnd===before.end&&note.selectionDirection===before.direction&&note.scrollTop===before.scroll,
 'Native finalization changed composing state: '+JSON.stringify({sameEditor:document.querySelector('.live-notes')===note,focused:document.activeElement===note.contentDOM,
 valueLength:note.value.length,expectedLength:before.value.length,selection:[note.selectionStart,note.selectionEnd,note.selectionDirection],expectedSelection:[before.start,before.end,before.direction],scroll:note.scrollTop,expectedScroll:before.scroll}));
await qa.complete(pending.request_id);
await until(()=>qa.root.querySelector('[data-request-id="'+pending.request_id+'"]').textContent.includes('Native synthetic answer'),'The original native answer did not finish after Stop.');
qa.failSaves=false;
note.contentDOM.dispatchEvent(new CompositionEvent('compositionend',{bubbles:true,data:'samengesteld'}));
qa.root.querySelector('.live-save').click();
await until(()=>qa.root.querySelector('.live-save-status').dataset.saveState==='saved','The stopped native draft could not be saved.');
const detail=await real('meeting_detail',{id:qa.id});assert(detail.notes===note.value,'The recovered native draft was not persisted by the real backend.');
return {stop_dispatched_before_save:true,stop_api_without_editor_blur:true,finalization_composition_focus_preserved:true,answer_completed_after_stop:true,dirty_editor_retained:true,real_backend_recovery_saved:true,
 stop_to_synthetic_capture_off_ms:stopped,actual_hardware_capture:false};
"""


def checkpoint(session, body, name):
    request = "native95-" + uuid.uuid4().hex
    script = REPORT.replace("__BODY__", body).replace("__REQUEST__", json.dumps(request))
    session.run("--capture-eval", "main", "(async()=>{" + script + "})()")
    deadline = time.monotonic() + 15
    latest = None
    while time.monotonic() < deadline:
        if session.process.poll() is not None:
            raise RuntimeError("The owned native app exited during " + name)
        try:
            latest = json.loads((session.data / "capture-state-main.json").read_text())
            if latest.get("request_id") == request:
                if latest.get("error"):
                    raise RuntimeError(name + ": " + latest["error"])
                return json.loads(latest.get("selected_tab") or "null")
        except (FileNotFoundError, json.JSONDecodeError):
            pass
        time.sleep(0.05)
    raise RuntimeError("Native checkpoint timed out: " + name)


def image(session, destination):
    snapshot = session.inspect("main")
    identifier = session.window("main")
    destination.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(["/usr/sbin/screencapture", "-x", "-o", "-l", str(identifier), str(destination)],
                   check=True, stdout=session.log, stderr=subprocess.STDOUT, timeout=12)
    if not destination.is_file() or not destination.stat().st_size:
        raise RuntimeError("The owned native window screenshot is empty")
    snapshot.pop("request_id", None)
    snapshot.update(window_id=identifier, native_window=session.describe_window("main"),
                    horizontal_overflow=snapshot["scroll_width"] > snapshot["viewport"][0] + 1)
    if snapshot["document_language"] != destination.parent.parent.name or snapshot["appearance"] != destination.parent.name:
        raise RuntimeError("The native locale or appearance differs from " + str(destination))
    if snapshot["horizontal_overflow"]:
        raise RuntimeError("Horizontal overflow in " + destination.name)
    return snapshot


def hide_and_reopen(session, bundle):
    pid = session.process.pid
    identifier = session.window("main")
    checkpoint(session, "await real('plugin:window|close',{label:'main'});return {close_requested:true};", "native close")
    deadline = time.monotonic() + 8
    while time.monotonic() < deadline:
        try:
            session.window("main")
        except subprocess.CalledProcessError:
            break
        time.sleep(0.05)
    else:
        raise RuntimeError("The owned native window did not hide after Close")
    subprocess.run(["open", "-a", str(bundle)], check=True, stdout=session.log, stderr=subprocess.STDOUT, timeout=15)
    session.inspect("main")
    if session.process.pid != pid or session.window("main") != identifier:
        raise RuntimeError("Finder reopen replaced the owned process or native window")
    result = checkpoint(session, LIVE_REOPEN, "native Finder reopen")
    return {**result, "native_close_hidden": True, "finder_reopen_same_pid": True, "same_native_window_id": True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=capture.DEFAULT_BINARY)
    parser.add_argument("--window-helper", type=Path, required=True)
    parser.add_argument("--output", type=Path, default=ROOT / "build/issue95/native-acceptance")
    parser.add_argument("--locale", choices=("both", "en", "nl"), default="both")
    parser.add_argument("--appearance", choices=("both", "light", "dark"), default="both")
    args = parser.parse_args()
    if platform.system() != "Darwin":
        raise SystemExit("Owned native acceptance requires macOS")
    binary = args.binary.resolve()
    helper = args.window_helper.resolve()
    capture.validate_binary(binary)
    if not helper.is_file():
        raise SystemExit("The compiled owned-window helper is missing")
    bundle = binary.parents[2]
    subprocess.run(["codesign", "--verify", "--deep", "--strict", str(bundle)], check=True)
    signature = subprocess.run(["codesign", "-d", "--verbose=4", str(bundle)], capture_output=True, text=True, check=True)
    if "TeamIdentifier=B538Q2K76C" not in signature.stderr:
        raise SystemExit("Native acceptance requires the personal Apple Developer team B538Q2K76C")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    manifest_path = output / "manifest.json"
    manifest = {"complete": False, "native_tauri_app": True, "capture_ipc": "synthetic", "provider_ipc": "synthetic",
                "actual_hardware_capture": False, "real_call_acceptance": False, "personal_team": "B538Q2K76C",
                "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "os_version": platform.mac_ver()[0],
                "screenshots": [], "checks": []}
    save_manifest = lambda: manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")
    save_manifest()
    locales = ("en", "nl") if args.locale == "both" else (args.locale,)
    appearances = ("light", "dark") if args.appearance == "both" else (args.appearance,)
    with tempfile.TemporaryDirectory(prefix="pennel-native95-") as temporary, (output / "app.log").open("w") as log:
        work = Path(temporary)
        backdrop = capture.NativeBackdrop(helper, work / "backdrop.json", log)
        backdrop.start()
        try:
            for locale in locales:
                for appearance in appearances:
                    data = work / (locale + "-" + appearance)
                    data.mkdir()
                    session = capture.CaptureSession(binary, data, log, helper, backdrop)
                    group = {"locale": locale, "appearance": appearance, "checks": {}}
                    manifest["checks"].append(group)
                    try:
                        session.initialize(locale, appearance)
                        capture.write_fixtures(data, locale)
                        session.launch("library")
                        session.inspect("main")
                        session.run("--capture-size", "main", "1400", "850")
                        group["checks"]["saved_meeting_settings"] = checkpoint(session, REAL_MEETING, "real saved meeting Settings")
                        relative = Path(locale) / appearance / "real-saved-meeting.png"
                        manifest["screenshots"].append({"locale": locale, "appearance": appearance, "file": str(relative),
                                                        "data_path": "real backend with isolated synthetic Markdown", **image(session, output / relative)})
                        for name, body in (("live_installed", INSTALL_LIVE), ("concurrency", LIVE_CONCURRENCY), ("live_settings", LIVE_SETTINGS)):
                            group["checks"][name] = checkpoint(session, body, name)
                            save_manifest()
                        group["checks"]["hide_finder_reopen"] = hide_and_reopen(session, bundle)
                        for width, height, name in ((1400, 850, "live-wide"), (760, 520, "live-minimum")):
                            session.run("--capture-size", "main", str(width), str(height))
                            time.sleep(0.35)
                            relative = Path(locale) / appearance / (name + ".png")
                            snapshot = image(session, output / relative)
                            if snapshot["document_language"] != locale:
                                raise RuntimeError("The native interface language differs from the requested language")
                            manifest["screenshots"].append({"locale": locale, "appearance": appearance, "file": str(relative), **snapshot})
                            save_manifest()
                        group["checks"]["failed_save_stop"] = checkpoint(session, LIVE_STOP_FAILURE, "native Stop with failed save")
                        relative = Path(locale) / appearance / "stopped-recovered.png"
                        manifest["screenshots"].append({"locale": locale, "appearance": appearance, "file": str(relative), **image(session, output / relative)})
                        save_manifest()
                        print(locale + "/" + appearance + " passed", flush=True)
                    except Exception as error:
                        group["failure"] = str(error)
                        save_manifest()
                        raise
                    finally:
                        session.stop()
        finally:
            backdrop.stop()
    manifest["complete"] = True
    save_manifest()


if __name__ == "__main__":
    main()
