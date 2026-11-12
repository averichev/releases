import { expect, it, vi } from 'vitest';
import { setCsrf, upload, ApiError } from './client';
class Xhr {
  static last: Xhr; headers: Record<string, string> = {}; upload = { onprogress: null as ((event: { lengthComputable: boolean; loaded: number; total: number }) => void) | null };
  onload?: () => void; onerror?: () => void; onabort?: () => void; status = 201; responseText = '{"id":"artifact"}'; withCredentials = false;
  open = vi.fn(); send = vi.fn(); setRequestHeader = (key: string, value: string) => { this.headers[key] = value; }; abort = () => this.onabort?.();
  constructor() { Xhr.last = this; }
}
it('uploads FormData directly with real progress and session protection', async () => {
  vi.stubGlobal('XMLHttpRequest', Xhr); setCsrf('upload-csrf'); const data = new FormData(); const file = new File(['abc'], 'installer.bin'); data.append('target', 'any'); data.append('file', file);
  const progress = vi.fn(); const promise = upload<{ id: string }>('/admin/files', data, new AbortController().signal, progress); const xhr = Xhr.last;
  expect(xhr.send).toHaveBeenCalledWith(data); expect(xhr.headers).toMatchObject({ 'X-CSRF-Token': 'upload-csrf', 'X-Veruna-Request': '1' }); expect(xhr.headers['Content-Type']).toBeUndefined(); expect(xhr.withCredentials).toBe(true);
  xhr.upload.onprogress?.({ lengthComputable: true, loaded: 25, total: 100 }); expect(progress).toHaveBeenCalledWith(25); xhr.onload?.(); expect(await promise).toEqual({ id: 'artifact' }); setCsrf();
});
it('supports cancellation and preserves API errors', async () => {
  vi.stubGlobal('XMLHttpRequest', Xhr); const controller = new AbortController(); const promise = upload('/admin/files', new FormData(), controller.signal, vi.fn()); controller.abort();
  await expect(promise).rejects.toMatchObject({ name: 'AbortError' });
  const failed = upload('/admin/files', new FormData(), new AbortController().signal, vi.fn()); Xhr.last.status = 413; Xhr.last.responseText = '{"error":{"code":"payload_too_large"}}'; Xhr.last.onload?.();
  await expect(failed).rejects.toBeInstanceOf(ApiError); await expect(failed).rejects.toMatchObject({ code: 'payload_too_large', status: 413 });
});
it('expires the session on an upload 401 and ignores stale unauthorized results', async () => {
  vi.stubGlobal('XMLHttpRequest', Xhr); const listener = vi.fn(); window.addEventListener('session-expired', listener); setCsrf('old');
  const old = upload('/admin/files', new FormData(), new AbortController().signal, vi.fn()); setCsrf('new'); Xhr.last.status = 401; Xhr.last.responseText = '{"error":{"code":"unauthorized"}}'; Xhr.last.onload?.();
  await expect(old).rejects.toBeInstanceOf(ApiError); expect(listener).not.toHaveBeenCalled();
  const current = upload('/admin/files', new FormData(), new AbortController().signal, vi.fn()); Xhr.last.status = 401; Xhr.last.responseText = '{"error":{"code":"unauthorized"}}'; Xhr.last.onload?.(); await expect(current).rejects.toBeInstanceOf(ApiError); expect(listener).toHaveBeenCalledOnce();
  window.removeEventListener('session-expired', listener); setCsrf();
});
