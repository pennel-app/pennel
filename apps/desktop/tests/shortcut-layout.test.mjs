import assert from 'node:assert/strict';
import test from 'node:test';
import { domShortcutKey, domBindingAvailable, keyboardLayout, refreshKeyboardLayout } from '../src/shortcut-layout.ts';
import { shortcutRow } from '../src/shortcut-presentation.ts';

test('layout normalization uses supplied base characters and never derives them from physical code names',()=>{
 const layout=new Map([['Digit1','1'],['KeyQ','a']]);
 assert.equal(domShortcutKey({key:'!',code:'Digit1',shiftKey:true},layout),'1');
 assert.equal(domShortcutKey({key:'æ',code:'KeyQ',altKey:true},layout),'a');
 assert.equal(domShortcutKey({key:'!',code:'Digit1',shiftKey:true},null),null);
 assert.equal(domShortcutKey({key:'!',code:'Digit1',shiftKey:true},new Map()),null);
 assert.equal(domShortcutKey({key:' ',code:'Space',altKey:true},null),'Space');
 assert.equal(domShortcutKey({key:'\u00a0',code:'Space',altKey:true},null),'Space');
 assert.equal(domShortcutKey({key:'F5',code:'F5',shiftKey:true},null),'F5');
 assert.equal(domShortcutKey({key:'a',code:'KeyQ',ctrlKey:true},null),'a');
});
test('DOM-only application availability fails closed for transformed printable bindings without clearing them',async()=>{
 await refreshKeyboardLayout(null);
 const binding={mods:['ctrl','shift'],code:'Digit1',key:'1'};
 assert.equal(domBindingAvailable(binding,keyboardLayout()),false);
 const row={id:'find',scope:'application',binding,editable:true,capability:{available:true,reason:null}};
 const info={platform:'windows',shortcuts:[row]};
 assert.equal(shortcutRow(info,'find').capability.reason,'dom_layout_unavailable');
 assert.equal(info.shortcuts[0].capability.available,true);assert.equal(info.shortcuts[0].binding,binding);
 assert.equal(shortcutRow({...info,platform:'macos'},'find'),row);
 await refreshKeyboardLayout({getLayoutMap:async()=>new Map([['Digit1','1']])});
 assert.equal(shortcutRow(info,'find'),row);
 await refreshKeyboardLayout(null);
});
test('missing or rejected browser layout support remains unavailable and stale layout replies cannot win',async()=>{
 await refreshKeyboardLayout({getLayoutMap:async()=>{throw Error('Unavailable');}});assert.equal(keyboardLayout(),null);
 let release;const pending=new Promise(r=>{release=r;});const older=refreshKeyboardLayout({getLayoutMap:()=>pending});
 await refreshKeyboardLayout({getLayoutMap:async()=>new Map([['KeyQ','a']])});release(new Map([['KeyQ','q']]));await older;
 assert.equal(keyboardLayout().get('KeyQ'),'a');await refreshKeyboardLayout(null);
});
