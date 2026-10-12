// Document offscreen (raison CLIPBOARD) : le service worker n'a pas accès au
// presse-papiers. `navigator.clipboard` exige une page au premier plan ; un
// document offscreen n'en est pas une, d'où `execCommand('copy')`, la méthode
// que recommande la documentation de Chrome pour ce cas.

import type { OffscreenCopy } from '../shared/api';

chrome.runtime.onMessage.addListener((msg: OffscreenCopy, sender, reply) => {
  if (msg.target !== 'offscreen' || sender.id !== chrome.runtime.id) return false;
  if (msg.type === 'copy') {
    const area = document.getElementById('clipboard') as HTMLTextAreaElement;
    area.value = msg.text;
    area.select();
    const ok = document.execCommand('copy');
    area.value = '';
    reply(ok);
  }
  return false;
});
