import { expect, it, vi } from 'vitest';
import { api, setCsrf, ApiError } from './client';
it('sends cookies and CSRF through the request headers', async () => {
  const fetch = vi.fn().mockResolvedValue(new Response(null, { status: 204 })); vi.stubGlobal('fetch', fetch);
  setCsrf('session-csrf'); await api('/auth/logout', 'POST', {});
  const init = fetch.mock.calls[0]?.[1] as RequestInit;
  expect(init.credentials).toBe('same-origin'); expect(init.headers).toMatchObject({ 'X-CSRF-Token': 'session-csrf', 'X-Veruna-Request': '1' });
  expect(window.localStorage.length).toBe(0); expect(window.sessionStorage.length).toBe(0); setCsrf();
});
it('does not expire the session when the current password was entered incorrectly', async () => {
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(JSON.stringify({ error: { code: 'invalid_credentials' } }), { status: 401 })));
  const listener=vi.fn(); window.addEventListener('session-expired',listener);
  await expect(api('/auth/change-password','POST',{})).rejects.toBeInstanceOf(ApiError);
  expect(listener).not.toHaveBeenCalled(); window.removeEventListener('session-expired',listener);
});

it('does not let an old unauthorized response clear a newer session', async () => {
  let resolve!: (response: Response) => void;
  vi.stubGlobal('fetch', vi.fn(() => new Promise<Response>((done) => { resolve = done; })));
  const listener = vi.fn(); window.addEventListener('session-expired', listener);
  setCsrf('old-session'); const request = api('/admin/users'); setCsrf('new-session');
  resolve(new Response(JSON.stringify({ error: { code: 'unauthorized' } }), { status: 401 }));
  await expect(request).rejects.toBeInstanceOf(ApiError); expect(listener).not.toHaveBeenCalled();
  window.removeEventListener('session-expired', listener); setCsrf();
});

it('uploads icon bytes with the session CSRF headers', async () => {
  const fetch = vi.fn().mockResolvedValue(new Response(JSON.stringify({ icon_url: '/api/v1/admin/applications/id/icon?v=new' })));
  vi.stubGlobal('fetch', fetch); setCsrf('icon-csrf');
  const file = new Blob(['image-bytes'], { type: 'image/png' });
  await api('/admin/applications/id/icon', 'POST', file);
  const init = fetch.mock.calls[0]?.[1] as RequestInit;
  expect(init.body).toBe(file); expect(init.credentials).toBe('same-origin');
  expect(init.headers).toMatchObject({ 'Content-Type': 'image/png', 'X-CSRF-Token': 'icon-csrf', 'X-Veruna-Request': '1' });
  setCsrf();
});
