#!/usr/bin/env python3
"""Check durable speaker rows in the owned native caption/workspace fixture."""

import importlib.util
from pathlib import Path


SPEC = importlib.util.spec_from_file_location(
    "pennel_native_captions", Path(__file__).with_name("verify-meeting-captions-native.py")
)
captions = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(captions)
workspace = captions.workspace
replace_once = captions.replace_once

workspace.INSTALL_LIVE = replace_once(
    workspace.INSTALL_LIVE,
    "window.__PENNEL_NATIVE95__=qa;",
    """
qa.enrich=()=>qa.transcript.forEach((segment,index)=>{
 if(segment.utterance_id)return;
 Object.assign(segment,{utterance_id:'native22-utterance-'+index,
  source_id:index===1?'microphone':'system_audio',
  attribution:{kind:index===1?'unknown':'single',voices:index===1?[]:[1],self_voice:false,turns:[]},
  attribution_revision:1});
});qa.enrich();
window.__PENNEL_NATIVE95__=qa;
""",
)
workspace.INSTALL_LIVE = replace_once(
    workspace.INSTALL_LIVE,
    "pending:0,transcript_revision:qa.revision,",
    "pending:0,metadata_status:'valid',transcript_revision:qa.revision,",
)
workspace.INSTALL_LIVE = replace_once(
    workspace.INSTALL_LIVE,
    "const base=segment.speaker+':'+segment.start_ms;",
    "const base='utterance:'+segment.utterance_id;",
)
workspace.INSTALL_LIVE = replace_once(
    workspace.INSTALL_LIVE, "segment_keys:[base+':'+occurrence]", "segment_keys:[base]"
)
workspace.INSTALL_LIVE = replace_once(
    workspace.INSTALL_LIVE,
    "JSON.stringify([segment.start_ms,segment.speaker,segment.text.trim().replace(/\\p{White_Space}+/gu,' '),segment.failed,segment.speaker==='others'?segment.voice??null:null])",
    "JSON.stringify([segment.utterance_id,segment.source_id,segment.start_ms,segment.end_ms,segment.text.split(/\\p{White_Space}+/u).filter(Boolean).join(' '),segment.failed])",
)
workspace.LIVE_CONCURRENCY = replace_once(
    workspace.LIVE_CONCURRENCY,
    "const captionStarted=performance.now();",
    """
const first=root.querySelector('[data-segment-key="utterance:native22-utterance-0"]');
assert(first,'The native row did not use its durable utterance identity.');
const source=qa.transcript[0].source_id;
qa.transcript[0].speaker='me';qa.transcript[0].voice=null;
qa.transcript[0].attribution={kind:'mixed',voices:[1,2],self_voice:false,turns:[]};
qa.transcript[0].attribution_revision=2;qa.revision++;
const captionStarted=performance.now();
""",
)
workspace.LIVE_CONCURRENCY = replace_once(
    workspace.LIVE_CONCURRENCY,
    "qa.revision++;qa.preview=qa.previewFor(3,1,",
    "qa.enrich();qa.revision++;qa.preview=qa.previewFor(3,1,",
)
workspace.LIVE_CONCURRENCY = replace_once(
    workspace.LIVE_CONCURRENCY,
    "return {display_only_captions:true,",
    """
assert(root.querySelector('[data-segment-key="utterance:native22-utterance-0"]')===first,
 'A speaker update replaced the native transcript row.');
assert(qa.transcript[0].source_id===source,'A person correction changed the physical source.');
return {durable_row_retained:true,physical_source_retained:true,display_only_captions:true,
""",
)


if __name__ == "__main__":
    workspace.main()
