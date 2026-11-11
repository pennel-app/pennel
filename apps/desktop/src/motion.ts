/** Motion is presentation-only: callers keep logical state, focus and operation lifetimes. */
export function playMotion(element: HTMLElement, name: string): void {
  if (element.classList.contains(name)) return;
  element.classList.add(name);
  const finish = (event: AnimationEvent) => {
    if (event.target !== element || element.getAnimations({ subtree: true }).some(animation => animation.effect instanceof KeyframeEffect && animation.effect.target === element && animation instanceof CSSAnimation && animation.playState === 'running')) return;
    element.classList.remove(name);
    element.removeEventListener('animationend', finish);
    element.removeEventListener('animationcancel', finish);
  };
  element.addEventListener('animationend', finish);
  element.addEventListener('animationcancel', finish);
}

/** A closed companion has already returned focus. Its inert shadow copy cannot enter app queries. */
export function companionExit(element: HTMLElement, owner: HTMLElement): () => void {
  const bounds = element.getBoundingClientRect();
  const host = document.createElement('div');
  host.className = 'motion-companion-outgoing';
  host.inert = true;
  host.setAttribute('aria-hidden', 'true');
  Object.assign(host.style, { position: 'fixed', left: `${bounds.left}px`, top: `${bounds.top}px`, width: `${bounds.width}px`, height: `${bounds.height}px`, zIndex: '6', pointerEvents: 'none' });
  const shadow = host.attachShadow({ mode: 'open' });
  for (const style of document.querySelectorAll('style,link[rel="stylesheet"]')) shadow.append(style.cloneNode(true));
  const wrapper = document.createElement('div');
  wrapper.className = owner.className;
  const copy = element.cloneNode(true) as HTMLElement;
  for (const node of [copy, ...copy.querySelectorAll<HTMLElement>('*')]) {
    for (const attribute of [...node.attributes]) if (/^(id|role|aria-live|aria-labelledby|aria-describedby|data-.+)$/.test(attribute.name)) node.removeAttribute(attribute.name);
  }
  Object.assign(copy.style, { position: 'absolute', inset: '0', margin: '0', width: '100%', height: '100%', boxSizing: 'border-box', animation: 'none' });
  wrapper.append(copy); shadow.append(wrapper); document.body.append(host);
  let timer: ReturnType<typeof setTimeout>;
  const remove = () => { clearTimeout(timer); host.remove(); };
  host.addEventListener('animationend', remove, { once: true });
  timer = setTimeout(remove, 250);
  return remove;
}
