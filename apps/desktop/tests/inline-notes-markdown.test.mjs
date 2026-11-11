import assert from 'node:assert/strict';
import test from 'node:test';
import {noteMarkdown} from '../src/inline-notes-markdown.ts';
const content=(source,line)=>source.slice(line.from,line.to);
const strong=(source,line)=>line.strong.map(mark=>source.slice(mark.from,mark.to));
test('headings, bullets and tasks describe stored UTF16 ranges without rewriting Markdown',()=>{
 const source='# Heading\n## Two **words**\n### Three\n- item\n* second\n- [ ] task\n- [x] done',lines=noteMarkdown(source);
 assert.deepEqual(lines.map(line=>line.heading),[1,2,3,0,0,0,0]);
 assert.equal(source.slice(lines[1].from,lines[1].prefixTo),'## ');assert.deepEqual(strong(source,lines[1]),['words']);
 assert.deepEqual(lines.slice(3).map(line=>line.marker.kind),['bullet','bullet','task','task']);assert.deepEqual(lines.slice(5).map(line=>line.marker.checked),[false,true]);
 assert.equal(source[lines[6].marker.checkAt],'x');assert.deepEqual(lines.map(line=>content(source,line)),source.split('\n'));
});
test('active lines keep their canonical marks and other lines still resolve',()=>{
 const source='## **Heading**\n- [x] **Task**\n**Other**',lines=noteMarkdown(source,[1,2]);
 assert.deepEqual(lines.map(line=>line.active),[true,true,false]);assert.deepEqual(strong(source,lines[0]),[]);assert.deepEqual(strong(source,lines[1]),[]);assert.deepEqual(strong(source,lines[2]),['Other']);
 assert.equal(content(source,lines[0]),'## **Heading**');assert.equal(lines[0].heading,2);
});
test('surrogates, combining marks and CRLF retain exact stored selection offsets',()=>{
 const source='😀 é\r\n## **naïef 👨‍👩‍👧**\r\n- [X] klaar',lines=noteMarkdown(source);
 assert.equal(lines[1].from,source.indexOf('##'));assert.equal(content(source,lines[1]),'## **naïef 👨‍👩‍👧**');
 const mark=lines[1].strong[0];assert.equal(source.slice(mark.from,mark.to),'naïef 👨‍👩‍👧');assert.equal(source.slice(mark.open,mark.open+2),'**');assert.equal(source.slice(mark.close,mark.close+2),'**');
 assert.equal(lines[2].from,source.indexOf('- [X]'));assert.equal(source[lines[2].marker.checkAt],'X');
});
test('fenced and indented code keep headings, tasks and bold literal',()=>{
 const source='# Real\n```md\n## literal\n**literal**\n```\n~~~\n- [x] literal\n~~~~\n    **indented**\n**shown**',lines=noteMarkdown(source);
 assert.deepEqual(lines.map(line=>line.code),[false,true,true,true,true,true,true,true,true,false]);
 assert.equal(lines[2].heading,0);assert.equal(lines[6].marker,null);assert.deepEqual(strong(source,lines[3]),[]);assert.deepEqual(strong(source,lines[8]),[]);assert.deepEqual(strong(source,lines[9]),['shown']);
});
test('an unclosed fence protects the remaining source, but unmatched inline ticks stay literal',()=>{
 const source='`unmatched **shown**\n```\n**not shown**\n- item',lines=noteMarkdown(source);
 assert.deepEqual(strong(source,lines[0]),['shown']);assert.equal(lines[2].code,true);assert.equal(lines[3].marker,null);
});
test('inline code spans protect Markdown including multi-line spans and exact tick runs',()=>{
 const source='`**code**` **yes**\n``a ` **literal** b`` **also**\n`multi\n**code**` **outside**',lines=noteMarkdown(source);
 assert.deepEqual(strong(source,lines[0]),['yes']);assert.deepEqual(strong(source,lines[1]),['also']);assert.deepEqual(strong(source,lines[2]),[]);assert.deepEqual(strong(source,lines[3]),['outside']);
});
test('multi-line code protects line prefixes and stops at a blank paragraph for LF and CRLF',()=>{
 for(const newline of ['\n','\r\n']){
  const source=['`multi','# literal','- [ ] literal`','','`open','','**shown**`'].join(newline),lines=noteMarkdown(source);
  assert.equal(lines[1].heading,0);assert.equal(lines[2].marker,null);assert.deepEqual(strong(source,lines[6]),['shown']);
 }
});
test('escaped delimiters and unsupported overlapping markers stay readable',()=>{
 const source=String.raw`\# literal\n\- literal\n\**not strong** **yes**\n***ambiguous***\n**one****two**`.replaceAll('\\n','\n');const lines=noteMarkdown(source);
 assert.equal(lines[0].heading,0);assert.equal(lines[1].marker,null);assert.deepEqual(strong(source,lines[2]),['yes']);assert.deepEqual(strong(source,lines[3]),[]);assert.deepEqual(strong(source,lines[4]),['one','two']);
});
test('empty and malformed syntax has no invented structure and ranges remain bounded',()=>{
 const source='\n#### not supported\n#no space\n- [z] not task\n** open\nclose **\n',lines=noteMarkdown(source);
 assert.equal(lines.length,7);assert.deepEqual(lines.map(line=>line.heading),Array(7).fill(0));assert.equal(lines[3].marker.kind,'bullet');assert.equal(lines[3].marker.checkAt,null);
 for(const line of lines){assert(line.from>=0&&line.to<=source.length&&line.from<=line.to);for(const mark of line.strong)assert(mark.open>=line.from&&mark.close+2<=line.to);}
});
