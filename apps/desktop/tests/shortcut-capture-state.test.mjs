import assert from 'node:assert/strict';
import test from 'node:test';
import { ShortcutCapture, shortcutCaptureActive } from '../src/shortcut-capture-state.ts';

const deferred = () => { let resolve; const promise = new Promise(r => { resolve=r; }); return {promise,resolve}; };
function fixture(start = async () => ({native:true,generation:7,reason:null})) {
  let handler; const bindings=[], states=[]; let stops=0, unlistens=0;
  const capture=new ShortcutCapture({start,stop:async()=>{stops++;},listen:async fn=>{handler=fn;return()=>{unlistens++;};},changed:s=>states.push(s),binding:b=>bindings.push(b)});
  return {capture,bindings,states,emit:e=>handler(e),get stops(){return stops;},get unlistens(){return unlistens;}};
}
const event=(patch={})=>({action:'dictate',generation:7,mods:['fn'],code:null,key:null,keycaps:['fn'],text:'fn',done:true,cancelled:false,...patch});
test('native completion preserves fn/sides and ignores unrelated or late events',async()=>{
 const f=fixture();await f.capture.start('dictate','global_hold');assert.equal(shortcutCaptureActive(),true);
 f.emit(event({generation:6}));f.emit(event({action:'find'}));assert.equal(f.bindings.length,0);
 f.emit(event({mods:['right_alt'],keycaps:['right ⌥']}));await f.capture.settled();
 assert.deepEqual(f.bindings,[{mods:['right_alt'],code:null,key:null}]);assert.equal(f.stops,1);assert.equal(f.unlistens,1);assert.equal(shortcutCaptureActive(),false);
 f.emit(event());assert.equal(f.bindings.length,1);
});
test('cancel during delayed activation restores backend after the late start and saves nothing',async()=>{
 const d=deferred(),f=fixture(()=>d.promise);const starting=f.capture.start('dictate','global_hold');await Promise.resolve();const stopping=f.capture.cancel();
 d.resolve({native:true,generation:9,reason:null});await Promise.all([starting,stopping]);
 assert.equal(f.stops,1);assert.equal(f.bindings.length,0);assert.equal(f.unlistens,1);assert.equal(shortcutCaptureActive(),false);
});
test('events delivered before activation reply are checked against its generation',async()=>{
 const d=deferred(),f=fixture(()=>d.promise);const starting=f.capture.start('dictate','global_hold');await Promise.resolve();await Promise.resolve();
 f.emit(event());d.resolve({native:true,generation:7,reason:null});await starting;await f.capture.settled();assert.equal(f.bindings.length,1);
});
test('DOM fallback is explicit, application-only, compositional-safe and never records bare fn',async()=>{
 const f=fixture(async()=>({native:false,generation:7,reason:'native_capture_unavailable'}));await f.capture.start('find','application');
 assert.equal(f.capture.keydown({key:'Fn',code:'Fn'}),false);assert.equal(f.capture.keydown({key:'a',code:'KeyQ',ctrlKey:true,isComposing:true}),false);
 assert.equal(f.capture.keydown({key:'a',code:'KeyQ',ctrlKey:true}),true);await f.capture.settled();
 assert.deepEqual(f.bindings,[{mods:['ctrl'],code:'KeyQ',key:'a'}]);
 const global=fixture(async()=>({native:false,generation:8,reason:'native_capture_unavailable'}));await global.capture.start('dictate','global_hold');assert.equal(global.bindings.length,0);assert.equal(global.stops,1);assert.equal(global.states.at(-1).phase,'error');
});
test('Escape cancels native or DOM capture without replacing the saved binding',async()=>{
 const f=fixture();await f.capture.start('dictate','global_hold');f.emit(event({cancelled:true,mods:[],keycaps:[]}));await f.capture.settled();assert.equal(f.bindings.length,0);
 const dom=fixture(async()=>({native:false,generation:8,reason:'accessibility_required'}));await dom.capture.start('find','application');assert.equal(dom.capture.keydown({key:'Escape',code:'Escape'}),true);await dom.capture.settled();assert.equal(dom.bindings.length,0);assert.equal(shortcutCaptureActive(),false);
});
test('a completed capture can await idempotent cancellation before its central settings write',async()=>{
 let handler,saved=false;const capture=new ShortcutCapture({start:async()=>({native:true,generation:7,reason:null}),stop:async()=>{},listen:async fn=>{handler=fn;return()=>{};},changed:()=>{},binding:async()=>{await capture.cancel();saved=true;}});
 await capture.start('dictate','global_hold');handler(event());
 await Promise.race([capture.settled(),new Promise((_,reject)=>setTimeout(()=>reject(Error('completion deadlocked before writer')),300))]);assert.equal(saved,true);
});
