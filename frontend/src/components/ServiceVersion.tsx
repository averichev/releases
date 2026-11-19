import { useEffect, useState } from 'react';
import { getHealth } from '../api/health';

export function ServiceVersion() {
  const [version, setVersion] = useState<string | null>(null);

  useEffect(() => {
    const controller = new AbortController();
    getHealth(controller.signal).then(
      (health) => { if (!controller.signal.aborted) setVersion(health.version); },
      () => { if (!controller.signal.aborted) setVersion('недоступна'); },
    );
    return () => controller.abort();
  }, []);

  return <span className="service-version" title="Версия платформы распространения" aria-live="polite">Версия {version ?? '…'}</span>;
}
