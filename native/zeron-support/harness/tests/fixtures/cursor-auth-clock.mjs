// Opt-in live-test preload. Advance only this test process's clock and record
// token exchange timing, never credentials or response bodies.
import fs from 'node:fs';
const control = process.env.ZERON_CURSOR_AUTH_CLOCK;
if (!control) throw new Error('Set an isolated ZERON_CURSOR_AUTH_CLOCK file');
const now = Date.now.bind(Date);
Date.now = () => {
  try { return now() + JSON.parse(fs.readFileSync(control, 'utf8')).offsetMs; }
  catch { return now(); }
};
const fetchOriginal = globalThis.fetch;
globalThis.fetch = async (...args) => {
  const response = await fetchOriginal(...args);
  if (String(args[0]).includes('/auth/exchange_user_api_key') && response.ok) {
    const data = await response.clone().json();
    const payload = JSON.parse(Buffer.from(data.accessToken.split('.')[1], 'base64url').toString());
    fs.appendFileSync(control + '.exchanges', JSON.stringify({pid: process.pid, at: now(), expiresAt: payload.exp * 1000}) + '\n');
  }
  return response;
};
