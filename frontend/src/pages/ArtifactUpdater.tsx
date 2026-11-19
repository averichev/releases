import { useState } from 'react';
import { Button, Select, Space, Tag } from 'antd';
import { api } from '../api/client';
import { artifactsPath, releaseError, type Artifact } from '../api/releases';
export default function ArtifactUpdater({ app, release, artifact, editable, disabled, onChange, onError }: { app: string; release: string; artifact: Artifact; editable: boolean; disabled: boolean; onChange: () => Promise<void>; onError: (error: string) => void }) {
  const [busy, setBusy] = useState(false);
  const path = `${artifactsPath(app, release)}/${artifact.id}`;
  async function change(role?: string, file?: File, remove = false) {
    if (file && (file.size > 16384 || !file.name.toLowerCase().endsWith('.sig'))) { onError('Выберите файл .sig до 16 КиБ.'); return; }
    setBusy(true); onError('');
    try {
      await api(role ? `${path}/updater` : `${path}/signature`, remove ? 'DELETE' : 'PUT', role ? { distribution_role: role } : file);
      await onChange();
    } catch (e) { onError(releaseError(e)); } finally { setBusy(false); }
  }
  return <Space orientation="vertical">
    {editable ? <Select id={`artifact-role-${artifact.id}`} aria-label={`Назначение ${artifact.filename}`} value={artifact.distribution_role ?? 'installer'} disabled={disabled || busy} onChange={(role) => { void change(role); }} options={[{ value: 'installer', label: 'Установочный пакет' }, { value: 'updater', label: 'Пакет обновления (Tauri Updater)' }]} /> : <Tag>{artifact.distribution_role === 'updater' ? 'Пакет обновления (Tauri Updater)' : 'Установочный пакет'}</Tag>}
    {artifact.distribution_role === 'updater' && <><Tag color={artifact.signature_present ? 'green' : 'orange'}>{artifact.signature_present ? 'Подпись прикреплена' : 'Нет подписи'}</Tag>
      {editable && <><label className="signature-picker">{artifact.signature_present ? 'Заменить .sig' : 'Добавить .sig'}<input aria-label={`Подпись ${artifact.filename}`} type="file" accept=".sig" disabled={disabled || busy} onChange={(event) => { const file = event.target.files?.[0]; event.target.value = ''; if (file) void change(undefined, file); }} /></label>{artifact.signature_present && <Button size="small" disabled={disabled || busy} onClick={() => { void change(undefined, undefined, true); }}>Удалить подпись</Button>}</>}
    </>}
  </Space>;
}
