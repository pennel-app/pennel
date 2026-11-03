#!/usr/bin/env python3
"""Exercise explicit live-speaker controls in the owned native workspace fixture."""

import importlib.util
from pathlib import Path


SPEC = importlib.util.spec_from_file_location(
    "pennel_native_turns", Path(__file__).with_name("verify-speaker-turns-native.py")
)
turns = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(turns)
workspace = turns.workspace
replace_once = turns.replace_once

workspace.INSTALL_LIVE = replace_once(
    workspace.INSTALL_LIVE,
    "window.__PENNEL_NATIVE95__=qa;",
    """
qa.live={enabled:false,available:true,state:'off',reason_code:null,revision:1,
 model:'owned-fixture-speaker',destination:'local'};
qa.liveCommands=0;
for(const segment of qa.transcript)segment.attribution.provisional=true;
window.__PENNEL_NATIVE95__=qa;
""",
)
workspace.INSTALL_LIVE = replace_once(
    workspace.INSTALL_LIVE,
    "pending:0,metadata_status:'valid',",
    "pending:0,metadata_status:'valid',live_speaker_attribution:structuredClone(qa.live),",
)
workspace.INSTALL_LIVE = replace_once(
    workspace.INSTALL_LIVE,
    "if(command==='meeting_status')return status();",
    """
if(command==='set_live_speaker_attribution'){
 assert(args.sessionId===qa.session,'A speaker toggle crossed the native session scope.');
 qa.liveCommands++;qa.live.enabled=!!args.enabled;qa.live.state=args.enabled?'running':'off';qa.live.revision++;
 return structuredClone(qa.live);
}
if(command==='meeting_status')return status();
""",
)
workspace.INSTALL_LIVE = replace_once(
    workspace.INSTALL_LIVE,
    "const intercepted=new Set(['start_meeting','app_info',",
    "const intercepted=new Set(['start_meeting','set_live_speaker_attribution','app_info',",
)
workspace.LIVE_CONCURRENCY = replace_once(
    workspace.LIVE_CONCURRENCY,
    "const qa=window.__PENNEL_NATIVE95__,root=qa.root,note=qa.editor;",
    """
const qa=window.__PENNEL_NATIVE95__,root=qa.root,note=qa.editor;
const speakerToggle=root.querySelector('[data-live-speaker-toggle]');
assert(speakerToggle&&!speakerToggle.checked&&qa.liveCommands===0,'Native speaker processing did not default off.');
speakerToggle.click();
await until(()=>speakerToggle.checked&&!speakerToggle.disabled&&qa.liveCommands===1,'Explicit native speaker opt-in did not complete.');
""",
)
workspace.LIVE_CONCURRENCY = replace_once(
    workspace.LIVE_CONCURRENCY,
    "{kind:'mixed',voices:[1,2],self_voice:false,turns:[]}",
    "{kind:'mixed',voices:[1,2],self_voice:false,provisional:true,turns:[]}",
)
workspace.LIVE_CONCURRENCY = replace_once(
    workspace.LIVE_CONCURRENCY,
    "return {durable_row_retained:true,",
    "return {live_opt_in_default_off:true,explicit_toggle_only:true,actual_speaker_inference:false,durable_row_retained:true,",
)


if __name__ == "__main__":
    workspace.main()
