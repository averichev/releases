import { describe, expect, it, vi } from 'vitest';
import { getHealth } from './health';

describe('health API contract', () => {
  it('returns the actual server version', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(JSON.stringify({ status: 'ok', service: 'veruna-releases', version: '0.1.0' }))));
    expect(await getHealth(new AbortController().signal)).toEqual({ status: 'ok', service: 'veruna-releases', version: '0.1.0' });
  });
  it('rejects non-success status', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response('{}', { status: 503 })));
    await expect(getHealth(new AbortController().signal)).rejects.toThrow('HTTP 503');
  });
  it('rejects an unrelated JSON response', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response('{"status":"ok"}')));
    await expect(getHealth(new AbortController().signal)).rejects.toThrow('неожиданный ответ');
  });
});
