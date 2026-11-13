export const sessionReturnPath = '/session-return';

export function nativeSystemPath(path: string, schemes: readonly string[]): string {
  try {
    const url = new URL(path);
    const ownScheme = schemes.includes(url.protocol.slice(0, -1));
    const plain = !url.username && !url.password && !url.search && !url.hash;
    const sync = (url.hostname === 'sync' && (url.pathname === '' || url.pathname === '/')) || (!url.hostname && url.pathname === '/sync');
    if (ownScheme && sync && plain) return '/settings/sync';
    // The keyboard opens this link; native code starts the session and the page asks the user to swipe back.
    if (ownScheme && url.hostname === 'sessie' && url.pathname === '/start' && plain) return sessionReturnPath;
  } catch {}
  return path;
}
