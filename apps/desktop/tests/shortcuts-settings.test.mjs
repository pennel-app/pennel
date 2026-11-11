import test from 'node:test';
import assert from 'node:assert/strict';
import { settingsCategory, legacyWorkspaceRoute, parseWorkspaceRoute } from '../src/workspace-state.ts';
import { commandKeycaps } from '../src/shortcut-presentation.ts';
test('Shortcuts has its own Settings category and native/legacy deep links retain the target',()=>{
 assert.equal(settingsCategory('shortcuts'),'shortcuts');
 assert.equal(legacyWorkspaceRoute('settings','shortcuts').tab,'shortcuts');
 assert.deepEqual(parseWorkspaceRoute({section:'settings',tab:'shortcuts'}),{section:'settings',tab:'shortcuts',meeting_id:null,jump_ms:null});
 for(const tab of ['dictionary','styles'])assert.equal(settingsCategory(tab),'dictation');
});
test('derived Command keycaps follow the effective Dictate binding in canonical modifier order',()=>{
 const info={shortcuts:[{id:'dictate',binding:{mods:['ctrl','alt','meta'],code:'KeyQ',key:'a'},label:{keycaps:['⌃','⌥','⌘','A'],text:'⌃⌥⌘A'}}],platform:'macos'};
 assert.deepEqual(commandKeycaps(info),['⌃','⌥','⇧','⌘','A']);
 info.shortcuts[0]={id:'dictate',binding:null,label:{keycaps:[],text:'None'}};assert.deepEqual(commandKeycaps(info),[]);
 info.platform='windows';info.shortcuts[0]={id:'dictate',binding:{mods:['right_ctrl'],code:null,key:null},label:{keycaps:['Right Ctrl'],text:'Right Ctrl'}};assert.deepEqual(commandKeycaps(info),['Right Ctrl','Shift']);
});
