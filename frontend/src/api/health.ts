export interface Health {
  status: 'ok';
  service: 'veruna-releases';
  version: string;
}

export async function getHealth(signal: AbortSignal): Promise<Health> {
  const response = await fetch('/api/v1/health', {
    signal: AbortSignal.any([signal, AbortSignal.timeout(5000)]),
    headers: { Accept: 'application/json' },
    cache: 'no-store',
  });
  if (!response.ok) throw new Error(`Сервис недоступен (HTTP ${response.status})`);
  const data: unknown = await response.json();
  if (typeof data !== 'object' || data === null ||
    !('status' in data) || data.status !== 'ok' ||
    !('service' in data) || data.service !== 'veruna-releases' ||
    !('version' in data) || typeof data.version !== 'string') {
    throw new Error('Сервис вернул неожиданный ответ');
  }
  return { status: data.status, service: data.service, version: data.version };
}
