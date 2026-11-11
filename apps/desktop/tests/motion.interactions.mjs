const assert = (value, message) => { if (!value) throw Error(message); };
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
const frame = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
async function until(check, message) { for (let i = 0; i < 240; i++) { if (await check()) return; await pause(20); } throw Error(message); }
const finish = element => element.getAnimations({ subtree: true }).filter(a => a.effect.getTiming().iterations !== Infinity).forEach(a => a.finish());
function reduced() { return matchMedia('(prefers-reduced-motion: reduce)').matches || document.documentElement.dataset.reduceMotion === 'true'; }
const movement = /^(transform|translate|scale|rotate|width|height|maxHeight|gridTemplateColumns|clipPath)$/;
function proof(element, duration, moving = true, name) {
  const animations = element.getAnimations().filter(a => !name || a.animationName === name);
  const animation = animations.find(a => a.effect.getTiming().duration === duration);
  assert(animation, `No actual ${duration}ms animation on ${element.className || element.id}: ${animations.map(a => `${a.animationName}:${a.effect.getTiming().duration}`).join(',')}`);
  const frames = animation.effect.getKeyframes();
  const moves = frames.some(f => Object.keys(f).some(key => movement.test(key) && f[key] !== 'none'));
  assert(moves === moving, `Unexpected movement in ${animation.animationName}: ${JSON.stringify(frames)}`);
  assert(frames.some(f => 'opacity' in f) || moving, 'Reduced motion removed the real fade.');
  animation.pause(); animation.currentTime = duration / 2;
  if (!moving) { const opacity = Number(getComputedStyle(element).opacity); assert(opacity > 0 && opacity < 1, 'The actual fade has no visible intermediate opacity.'); }
  window.__PENNEL_CONTROL_ACCEPTANCE__ ??= {};
  window.__PENNEL_CONTROL_ACCEPTANCE__.motion ??= [];
  window.__PENNEL_CONTROL_ACCEPTANCE__.motion.push({ name: animation.animationName, duration, delay: animation.effect.getTiming().delay, movement: moves,
    opacity_at_midpoint: !moving ? Number(getComputedStyle(element).opacity) : null,
    keyframes: frames.map(({offset,opacity,transform,backgroundColor,color,gridTemplateColumns}) => ({offset,opacity,transform,backgroundColor,color,gridTemplateColumns})) });
  animation.play(); return animation;
}
export async function checkPillMotion() {
  const fixture = window.__PENNEL_FIXTURE__, pill = document.getElementById('pill');
  await until(() => !pill.hidden, 'Pill did not load.'); finish(pill); await frame();
  if (fixture.options.reduce_motion) { fixture.emit('desktop-environment-changed', { ...(await window.__PENNEL_PLATFORM__.invoke('desktop_environment')), reduce_motion: true }); await frame(); }
  fixture.emit('pill-state', { state: 'hidden' }); finish(pill); await frame();
  fixture.emit('pill-state', { state: 'listening' });
  const enter = proof(pill, reduced() ? 120 : 220, !reduced());
  if (!reduced()) assert(enter.effect.getKeyframes()[0].transform.includes('8px'), 'Pill no longer arrives from8px.');
  finish(pill); await frame();
  await import('/tests/pill.interactions.mjs').then(m => m.checkPillCrossfade());
  fixture.emit('pill-state', { state: 'hidden' }); proof(pill, reduced() ? 120 : 140, !reduced()); finish(pill); await frame();
  assert(pill.hidden, 'Reduced-motion leave failed to finish the real hide lifecycle.');
}
export async function checkMeetingMotion() {
  const root = document.querySelector('.live-workspace'), fixture = window.__PENNEL_FIXTURE__;
  await until(() => root?.querySelector('.meeting-companion') && window.__PENNEL_DESKTOP_COMMAND_STATE__?.ready, 'Meeting did not load.');
  const companion = root.querySelector('.meeting-companion'), note = root.querySelector('.live-notes');
  if (!companion.hidden) root.querySelector('.meeting-companion-close').click();
  finish(document.body); await frame();
  note.applyUserChange('**Synthetic Markdown**\nSecond line'); note.focus(); note.setSelectionRange(2, 4, 'backward');
  const saved = [note.value, note.selectionStart, note.selectionEnd, note.selectionDirection];
  const tab = root.querySelector('[data-live-tab="transcript"]'); tab.focus(); tab.click();
  const arrival = proof(companion, reduced() ? 120 : 220, !reduced());
  await pause(40);
  assert(companion.getAnimations().includes(arrival), 'An unchanged status restarted companion motion.');
  finish(root); await frame();
  root.querySelector('.meeting-companion-close').click();
  assert(companion.hidden && !root.querySelector('.meeting-primary-column').inert, 'Closing delayed the logical/focus state.');
  const outgoing = document.querySelector('.motion-companion-outgoing');
  assert(outgoing?.inert && outgoing.getAttribute('aria-hidden') === 'true' && !outgoing.shadowRoot.querySelector('[id],[role],[aria-live],[data-live-pane],[data-insight-id],[data-live-tab]'), 'Closing duplicated interactive or announcing semantics.');
  proof(outgoing, reduced() ? 120 : 130, !reduced()); finish(document.body); await frame();
  assert(!outgoing.isConnected && document.activeElement === note.contentDOM, 'Closing retained a snapshot or lost editor focus.');
  assert(JSON.stringify([note.value, note.selectionStart, note.selectionEnd, note.selectionDirection]) === JSON.stringify(saved), 'Companion motion altered the note draft/selection.');
  tab.click(); finish(root); await frame();
  root.querySelector('.meeting-companion-close').click();
  const stale = document.querySelector('.motion-companion-outgoing');
  tab.click();
  assert(!companion.hidden && !stale.isConnected && !document.querySelector('.motion-companion-outgoing'), 'Rapid reopen retained the obsolete close snapshot.');
  await pause(280);
  assert(!companion.hidden, 'An obsolete visual close timer hid a reopened companion.');
  finish(root); await frame();
  const initialRows = [...root.querySelectorAll('.transcript-line')], textNode = initialRows[0].querySelector('.transcript-copy').firstChild;
  const range = document.createRange(); range.setStart(textNode, 0); range.setEnd(textNode, Math.min(5, textNode.length)); getSelection().removeAllRanges(); getSelection().addRange(range);
  const selected = getSelection().toString(), output = root.querySelector('.live-transcript'); output.scrollTop = 0; output.dispatchEvent(new Event('scroll'));
  fixture.workspace.append({ speaker: 'me', start_ms: 900000, end_ms: 901000, text: 'Synthetic arrival' });
  await until(() => root.querySelectorAll('.transcript-line').length > initialRows.length, 'New transcript did not arrive.');
  const incoming = [...root.querySelectorAll('.transcript-line')].at(-1); proof(incoming, reduced() ? 120 : 320, !reduced());
  assert(initialRows.every((row, index) => root.querySelectorAll('.transcript-line')[index] === row) && getSelection().anchorNode === textNode && getSelection().toString() === selected && output.scrollTop === 0, 'Transcript arrival disturbed retained nodes, selection or paused scroll.');
  finish(root); await pause(1100);
  assert(!incoming.getAnimations().length, 'Unchanged transcript refresh restarted its arrival.');
  getSelection().removeAllRanges();
  root.querySelector('.meeting-companion-close').click(); finish(document.body); await frame();
  note.focus(); note.setSelectionRange(3, 3); finish(root); await frame(); note.setSelectionRange(note.value.length, note.value.length);
  const resolved = note.querySelector('.cm-line.motion-resolve'); assert(resolved, 'Leaving Markdown did not resolve its actual line.');
  if (reduced()) proof(resolved, 120, false); else assert(resolved.getAnimations().some(a => a.effect.getTiming().duration === 220), 'Markdown resolution lost220ms.');
  assert(note.value === saved[0], 'Markdown motion changed saved source bytes.');
}
export async function checkShortcutMotion() {
  await until(() => document.querySelector('[data-shortcut-record="find"]'), 'Shortcuts did not load.');
  const fixture = window.__PENNEL_FIXTURE__, record = document.querySelector('[data-shortcut-record="find"]');
  record.focus(); record.click(); await until(() => fixture.shortcutCapture()?.action === 'find', 'Capture did not start.'); await frame();
  assert(reduced() ? record.getAnimations().every(a => a.effect.getKeyframes().every(f => !f.transform)) : record.getAnimations().some(a => a.animationName === 'shortcut-waiting'), 'Waiting pulse ignores motion preference.');
  const session = fixture.shortcutCapture();
  fixture.emit('shortcut-capture', { ...session, mods: ['alt'], code: null, key: null, keycaps: ['⌥'], text: '⌥', done: false, cancelled: false });
  await until(() => record.querySelector('kbd'), 'Observed key did not appear.');
  const cap = record.querySelector('kbd'), animation = proof(cap, 120, !reduced());
  fixture.emit('shortcut-capture', { ...session, mods: ['alt'], code: null, key: null, keycaps: ['⌥'], text: '⌥', done: false, cancelled: false }); await frame();
  assert(record.querySelector('kbd') === cap && cap.getAnimations().includes(animation), 'Duplicate observed keys reanimate or replace the keycap.');
  fixture.emit('shortcut-capture', { ...session, mods: [], code: null, key: null, keycaps: [], text: '', done: true, cancelled: true });
  await until(() => !fixture.shortcutCapture(), 'Capture did not cancel.');
}
export async function checkSummaryMotion() {
  const reader = document.querySelector('.meeting-summary-structured');
  await until(() => reader && !reader.hidden && reader.querySelector('.summary-map-span'), 'Structured reading/timeline did not load.');
  const controls = document.querySelector('.meeting-summary-length');
  const buttons = [...controls.querySelectorAll('button')]; buttons[0].click(); finish(reader); await frame(); buttons[2].click();
  const topic = reader.querySelector('.structured-summary-topics');
  proof(topic, reduced() ? 120 : 320, !reduced());
  const map = reader.querySelector('.summary-map-span');
  proof(map, reduced() ? 120 : 220, !reduced());
  if (!reduced()) { const sections = [...reader.querySelectorAll('.motion-summary')].filter(n => !n.hidden); const delays = sections.map(n => n.getAnimations()[0]?.effect.getTiming().delay).filter(v => v !== undefined); assert(delays.some(v => v >= 90), 'Summary sections lost their actual90ms stagger.'); }
  finish(reader); await frame();
}
export async function checkShareRoles() {
  await import('/tests/formatted-export.interactions.mjs').then(m => m.renderExportReference());
  const dialog = document.querySelector('.meeting-export-dialog'), group = dialog.querySelector('.meeting-export-heading');
  assert(parseFloat(getComputedStyle(dialog.querySelector('h2')).fontSize) === 20, 'Share title does not use the20px title role.');
  assert(Math.abs(group.getBoundingClientRect().width - dialog.querySelector('.meeting-export-review').clientWidth + 8) < 3, 'Share scope does not fill its reading width.');
}

export async function checkHomeMotion() {
  const fixture = window.__PENNEL_FIXTURE__, client = window.__PENNEL_PLATFORM__, original = client.invoke.bind(client);
  await until(() => document.querySelector('[data-recent-id]'), 'Recent rows did not load.');
  const entry = { id: 'motion-new-row', created_at_ms: Date.now(), app_bundle_id: null, raw: 'Synthetic new result', text: 'Synthetic new result', status: 'done', error: null, has_audio: false };
  client.invoke = async (command, args) => command === 'history' ? [entry, ...await original(command,args)] : original(command,args);
  try {
    const old = document.querySelector('[data-recent-id]'); old.querySelector('.recent-dictation-copy').focus();
    fixture.emit('history-changed', null);
    await until(() => document.querySelector('[data-recent-id="motion-new-row"]'), 'The real history event did not add a row.');
    const row = document.querySelector('[data-recent-id="motion-new-row"]');
    proof(row, reduced() ? 120 : 320, !reduced(), reduced() ? 'desktop-fade' : 'home-result-arrives');
    if (!reduced()) assert(row.getAnimations().some(a => a.effect.getTiming().duration === 220), 'New row did not make room over220ms.');
    assert(row.getAnimations().some(a => a.animationName === 'home-result-tint' && a.effect.getTiming().duration === 1600), 'Arrival duration was conflated with the tint.');
    const arrival = row.getAnimations()[0]; fixture.emit('history-changed', null); await frame();
    assert(row.getAnimations().includes(arrival) && document.activeElement === old.querySelector('.recent-dictation-copy'), 'Unchanged history restarted arrival or lost focus.');
    const copy = row.querySelector('.recent-dictation-copy'); copy.focus(); copy.click();
    await until(() => copy.dataset.copied === 'true', 'Copy did not acknowledge.');
    proof(copy.firstElementChild, 120, !reduced());
    await pause(950); assert(copy.dataset.copied === 'true', 'Row Copy feedback ended before1200ms.');
    await until(() => !copy.dataset.copied, 'Row Copy feedback outlived its owned timer.');
  } finally { client.invoke = original; }
}
export async function checkReviewMotion() {
  const root = document.querySelector('.live-workspace');
  await until(() => root?.querySelector('.insight-card[data-review-state="proposed"]'), 'Review proposals did not load.');
  const card = root.querySelector('.insight-card[data-review-state="proposed"]'), count = root.querySelector('.review-count');
  const notes = window.__PENNEL_FIXTURE__.workspace.notes(), button = card.querySelector('.insight-accept');
  button.focus(); button.click(); await until(() => card.dataset.reviewState === 'accepted', 'Acceptance did not persist.');
  const animation = proof(card, reduced() ? 120 : 220, !reduced());
  proof(count, reduced() ? 120 : 220, !reduced());
  assert(document.activeElement === card.querySelector('.insight-undo') && window.__PENNEL_FIXTURE__.workspace.notes() === notes, 'Acceptance motion changed focus/notes ownership.');
  await frame(); assert(card.getAnimations().includes(animation), 'Unchanged review render restarted acceptance.');
  finish(root); await frame();
}
export async function checkReducedMotionDetector() {
  assert(reduced(), 'This negative control requires reduced motion.');
  const probe = document.createElement('span'); document.body.append(probe);
  const moving = probe.animate([{opacity:0,transform:'translateX(12px)'},{opacity:1,transform:'none'}],{duration:120});
  let rejected = false; try { proof(probe,120,false); } catch(error) { rejected = /Unexpected movement/.test(String(error)); }
  moving.cancel(); assert(rejected,'The keyframe checker accepted surviving movement.');
  const noFade = probe.animate([{color:'red'},{color:'blue'}],{duration:120}); rejected = false;
  try { proof(probe,120,false); } catch(error) { rejected = /real fade/.test(String(error)); }
  noFade.cancel(); probe.remove(); assert(rejected,'The keyframe checker accepted a missing active fade.');
  window.__PENNEL_CONTROL_ACCEPTANCE__ = { motion_negative_controls: { surviving_movement_rejected: true, missing_active_fade_rejected: true } };
}

export async function checkCapsuleMotion() {
  const capsule = document.querySelector('.toolbar-capture'), observed = [];
  const record = event => {
    if (event.target !== capsule) return;
    const animation = capsule.getAnimations().find(a => a.transitionProperty === event.propertyName);
    if (animation) observed.push({property: event.propertyName, duration: animation.effect.getTiming().duration, frames: animation.effect.getKeyframes()});
  };
  capsule.addEventListener('transitionrun',record);
  try { await import('/tests/recording-capsule.interactions.mjs').then(m => m.checkRecordingCapsule()); }
  finally { capsule.removeEventListener('transitionrun',record); }
  window.__PENNEL_CONTROL_ACCEPTANCE__ = { capsule_motion: observed };
  assert(observed.some(a => a.property === 'background-color' && a.duration === 120 && a.frames[0].backgroundColor !== a.frames.at(-1).backgroundColor), 'Capsule has no actual120ms colour fade.');
  if (reduced()) assert(!observed.some(a => a.property === 'width'), 'Reduced capsule still animates width.');
  else for (const duration of [220,130]) assert(observed.some(a => a.property === 'width' && a.duration === duration), `Capsule did not really resize over${duration}ms.`);
}
export async function checkSummaryPlaceholders() {
  await until(() => document.querySelector('[data-summary-consent="create"]:not(:disabled)'), 'Summary consent did not become ready.');
  document.querySelector('[data-summary-consent="create"]').click();
  const placeholders = document.querySelector('.summary-placeholders');
  await until(() => !placeholders.hidden, 'The actual synthetic request did not reveal placeholders.');
  const line = placeholders.firstElementChild;
  if (reduced()) proof(line,120,false); else assert(line.getAnimations().some(a => a.effect.getTiming().duration === 1200 && a.effect.getTiming().iterations === Infinity), 'Working placeholders did not pulse.');
  await until(() => placeholders.hidden && document.querySelector('.meeting-summary-markdown .markdown'), 'Summary request did not reach reading.');
  const sections = [...document.querySelectorAll('.meeting-summary-markdown .markdown > *')];
  assert(sections.length > 1, 'The summary fixture must contain multiple actual sections.');
  const animation = proof(sections[0],reduced()?120:320,!reduced());
  if (!reduced()) assert(sections[1].getAnimations()[0]?.effect.getTiming().delay === 90, 'Generated summary sections are not90ms apart.');
  await frame(); assert(sections[0].getAnimations().includes(animation), 'Unchanged summary status restarted its arrival.');
  finish(document.body); await frame();
}

export async function checkSidebarMotion() {
  await until(() => window.__PENNEL_DESKTOP_COMMAND_STATE__?.ready, 'Sidebar did not become ready.');
  const target = document.querySelector('.library-shortcuts > [data-workspace-section="meetings"]');
  target.focus(); target.click();
  await until(() => target.getAttribute('aria-current') === 'page', 'Task navigation did not finish.');
  const animation = target.getAnimations({subtree:true}).find(a => a.effect.getTiming().duration === (reduced() ? 120 : 220));
  assert(animation, `Actual sidebar selection did not slide/fade: ${target.className}; ${getComputedStyle(target,'::before').animationName}; ${target.getAnimations({subtree:true}).map(a=>a.animationName+':'+a.effect.getTiming().duration)}`);
  const frames = animation.effect.getKeyframes();
  assert(reduced() ? frames.every(f => !f.transform) : frames.some(f => f.transform && f.transform !== 'none'), 'Sidebar selection ignored reduced motion.');
  if (reduced()) { animation.pause(); animation.currentTime = 60; const opacity = Number(getComputedStyle(target,'::before').opacity); assert(opacity > 0 && opacity < 1, 'The sidebar fade has no active intermediate opacity.'); animation.play(); }
  window.__PENNEL_CONTROL_ACCEPTANCE__ = { sidebar_motion: { name: animation.animationName, duration: animation.effect.getTiming().duration, keyframes: frames, reduced: reduced() } };
  finish(document.body); await frame();
}
