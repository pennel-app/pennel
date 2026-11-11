import test from 'node:test';
import assert from 'node:assert/strict';
import { shortcutText, shortcutKeycaps } from '../src/shortcut-presentation.ts';
import { setInterfaceLanguage } from '../src/i18n.ts';
test('every shortcut label follows effective core rows, including cleared and AZERTY bindings',()=>{
 const info={platform:'macos',settings:{hotkey:'fn'},shortcuts:[{id:'dictate',binding:{mods:['ctrl','alt'],code:'KeyQ',key:'a'},label:{text:'⌃⌥A',keycaps:['⌃','⌥','A']}}]};
 assert.equal(shortcutText(info,'dictate'),'⌃⌥A');assert.deepEqual(shortcutKeycaps(info,'dictate'),['⌃','⌥','A']);
 info.shortcuts[0]={...info.shortcuts[0],binding:null,label:{text:'None',keycaps:[]}};setInterfaceLanguage('nl');
 assert.equal(shortcutText(info,'dictate'),'Geen');assert.deepEqual(shortcutKeycaps(info,'dictate'),[]);
});
