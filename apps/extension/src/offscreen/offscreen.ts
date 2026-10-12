// Document offscreen (raison CLIPBOARD) : le service worker n'a pas accès au
// presse-papiers. `navigator.clipboard` exige une page au premier plan ; un
// document offscreen n'en est pas une, d'où `execCommand`, la méthode que
// recommande la documentation de Chrome pour ce cas. La lecture (`paste`)
// demande la permission `clipboardRead` et ne sert qu'au raccourci.

import type { OffscreenMessage } from '../shared/api';

chrome.runtime.onMessage.addListener((msg: OffscreenMessage, sender, reply) => {
  if (msg.target !== 'offscreen' || sender.id !== chrome.runtime.id) return false;
  const area = document.getElementById('clipboard') as HTMLTextAreaElement;
  if (msg.type === 'copy') {
    area.value = msg.text;
    area.select();
    const ok = document.execCommand('copy');
    area.value = '';
    reply(ok);
  } else {
    area.value = '';
    area.focus();
    const text = document.execCommand('paste') ? area.value : null;
    area.value = '';
    reply(text);
  }
  return false;
});
