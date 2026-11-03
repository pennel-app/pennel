#!/usr/bin/env python3
"""Exercise display-only captions in the owned native #95 workspace harness."""

import importlib.util
from pathlib import Path


SPEC = importlib.util.spec_from_file_location(
    "pennel_native_workspace", Path(__file__).with_name("verify-live-workspace-native.py")
)
workspace = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(workspace)


def replace_once(source, original, replacement):
    if source.count(original) != 1:
        raise RuntimeError("The owned workspace fixture changed; review caption adaptation.")
    return source.replace(original, replacement, 1)


workspace.INSTALL_LIVE = replace_once(
    workspace.INSTALL_LIVE,
    "window.__PENNEL_NATIVE95__=qa;",
    """
qa.previewFor=(revision,epoch,microphone,system)=>({session_id:'native17-preview-session',revision,
 transcript_revision:qa.revision,tracks:['me','others'].map((source,index)=>({source,revision,epoch,
 chunk_start_ms:14000,start_ms:14000,end_ms:21000,mode:'local',state:'ready',
 text:index?system:microphone}))});
qa.preview=qa.previewFor(1,0,'Synthetic microphone interim caption.','Synthetic computer interim caption.');
window.__PENNEL_NATIVE95__=qa;
""",
)
workspace.INSTALL_LIVE = replace_once(
    workspace.INSTALL_LIVE,
    "pending:0,transcript_revision:qa.revision,",
    "pending:0,transcript_revision:qa.revision,preview:structuredClone(qa.preview),",
)

workspace.LIVE_CONCURRENCY = replace_once(
    workspace.LIVE_CONCURRENCY,
    "const before={value:note.value",
    """
const microphone=root.querySelector('[data-preview-source="me"]');
const computer=root.querySelector('[data-preview-source="others"]');
await until(()=>microphone?.textContent.includes('Synthetic microphone interim'),'Native captions did not render.');
const captionStarted=performance.now();
qa.preview=qa.previewFor(2,0,'New synthetic microphone interim.','New synthetic computer interim.');
await until(()=>computer.textContent==='New synthetic computer interim.','Native preview-only update did not render.');
assert(root.querySelector('[data-preview-source="me"]')===microphone,'Native preview replaced its stable node.');
assert(document.activeElement===note,'Native partial text stole composition focus.');
qa.captionLatency=performance.now()-captionStarted;
const before={value:note.value""",
)
workspace.LIVE_CONCURRENCY = replace_once(
    workspace.LIVE_CONCURRENCY,
    "qa.revision++;",
    "qa.revision++;qa.preview=qa.previewFor(3,1,'Next synthetic microphone caption.','Next synthetic computer caption.');",
)
workspace.LIVE_CONCURRENCY = replace_once(
    workspace.LIVE_CONCURRENCY,
    "return {same_editor:true,",
    "return {display_only_captions:true,stable_caption_nodes:true,preview_update_ms:qa.captionLatency,same_editor:true,",
)

original_reopen = workspace.hide_and_reopen


def captions_after_reopen(session, bundle):
    result = original_reopen(session, bundle)
    captions = workspace.checkpoint(
        session,
        """
const qa=window.__PENNEL_NATIVE95__,root=qa.root;
tab(root,'transcript');
await until(()=>root.querySelector('[data-preview-source="others"]')?.textContent==='Next synthetic computer caption.',
 'Reopening lost the current synthetic caption.');
assert(!root.querySelector('.live-transcript').textContent.includes('interim'),'Interim text entered finalized rows.');
return {caption_reopen:true,interim_excluded_from_finalized_rows:true};
""",
        "caption reopen",
    )
    return {**result, **captions}


workspace.hide_and_reopen = captions_after_reopen


if __name__ == "__main__":
    workspace.main()
