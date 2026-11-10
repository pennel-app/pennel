import assert from 'node:assert/strict';
import test from 'node:test';
import {EditorSelection,Transaction} from '@codemirror/state';
import {redo,undo,undoDepth} from '@codemirror/commands';
import {formatNote,inlineNoteState,noteSelection,quietNoteChange,quietNoteValue,userNoteValue} from '../src/inline-notes-state.ts';
test('the editor model preserves exact canonical newlines and UTF16 selections',()=>{
 const value='## 😀 é\r\n- [ ] task\n';let state=inlineNoteState(value);assert.equal(state.doc.toString(),value);
 state=state.update({selection:noteSelection(3,8,'backward',value.length)}).state;
 const change=quietNoteValue(state,value+'Tail');assert.ok(change);const next=state.update(change);
 assert.equal(next.state.doc.toString(),value+'Tail');assert.equal(next.state.selection.main.anchor,8);assert.equal(next.state.selection.main.head,3);
 assert.equal(next.annotation(quietNoteChange),true);assert.equal(next.annotation(Transaction.addToHistory),false);assert.equal(next.scrollIntoView,false);assert.equal(undoDepth(next.state),0);
});
test('toolbar formatting is one genuine isolated edit with an exact undo and redo',()=>{
 let state=inlineNoteState('Before 😀 words.');state=state.update({selection:EditorSelection.single(7,15)}).state;
 const before=state.doc.toString(),edit=formatNote(state,'bold');assert.ok(edit);let transaction=state.update(edit);state=transaction.state;
 assert.equal(state.doc.toString(),'Before **😀 words**.');assert.equal(transaction.isUserEvent('input'),true);assert.notEqual(transaction.annotation(quietNoteChange),true);assert.equal(undoDepth(state),1);
 const target={get state(){return state;},dispatch(tr){state=tr.state;}};assert.equal(undo(target),true);assert.equal(state.doc.toString(),before);assert.equal(redo(target),true);assert.equal(state.doc.toString(),'Before **😀 words**.');
});
test('line formatting and user replacements retain canonical source offsets and history',()=>{
 let state=inlineNoteState('One\nTwo');state=state.update({selection:EditorSelection.single(1,6)}).state;
 state=state.update(formatNote(state,'task')).state;assert.equal(state.doc.toString(),'- [ ] One\n- [ ] Two');assert.equal(state.selection.main.from,7);
 const value=state.doc.toString()+'\n@Owner';const edit=userNoteValue(state,value,{anchor:value.length-6,head:value.length});assert.ok(edit);state=state.update(edit).state;
 assert.equal(state.doc.toString(),value);assert.equal(state.selection.main.to,value.length);assert.equal(undoDepth(state),2);
 assert.equal(quietNoteValue(state,value),null);assert.equal(userNoteValue(state,value),null);
});
