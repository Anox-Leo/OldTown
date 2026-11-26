import type { OffscreenMessage } from '../shared/api';

chrome.runtime.onMessage.addListener((msg: OffscreenMessage, sender, reply) => {
  if (msg.target !== 'offscreen' || sender.id !== chrome.runtime.id) return false;
  if (msg.type === 'blob') {
    void fileBlobUrl(msg.url).then(reply, () => reply(null));
    return true;
  }
  if (msg.type === 'revoke') {
    URL.revokeObjectURL(msg.url);
    reply(true);
    return false;
  }
  const area = document.getElementById('clipboard') as HTMLTextAreaElement;
  if (msg.type === 'copy') {
    area.value = msg.text;
    area.select();
    const ok = document.execCommand('copy');
    area.value = '';
    reply(ok);
  } else if (msg.type === 'read') {
    area.value = '';
    area.focus();
    const text = document.execCommand('paste') ? area.value : null;
    area.value = '';
    reply(text);
  }
  return false;
});

async function fileBlobUrl(url: string): Promise<string> {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return URL.createObjectURL(await res.blob());
}
