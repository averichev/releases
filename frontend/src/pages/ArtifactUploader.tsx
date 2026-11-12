import { useEffect, useRef, useState } from 'react';
import { InboxOutlined } from '@ant-design/icons';
import { Alert, Button, Form, Input, Progress, Select, Space, Typography, Upload } from 'antd';
import type { UploadFile } from 'antd';
import { upload } from '../api/client';
import { artifactsPath, detectedType, releaseError, size, targetName, type Artifact, type UploadOptions } from '../api/releases';
export default function ArtifactUploader({ applicationId, releaseId, artifacts, options, onChange, onBusy }: { applicationId: string; releaseId: string; artifacts: Artifact[]; options: UploadOptions; onChange: () => void; onBusy: (busy: boolean) => void }) {
  const [file, setFile] = useState<File>(); const [target, setTarget] = useState<string>(); const [format, setFormat] = useState('');
  const [busy, setBusy] = useState(false); const [percent, setPercent] = useState(0); const [error, setError] = useState(''); const [notice, setNotice] = useState('');
  const controller = useRef<AbortController | null>(null); const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; controller.current?.abort(); }; }, []);
  async function send() {
    if (controller.current || !file || !target) return;
    const fileType = format.trim().toLowerCase() || detectedType(file.name);
    setError(''); setNotice('');
    if (file.size === 0 || file.size > options.max_file_bytes) { setError(`Выберите непустой файл до ${size(options.max_file_bytes)}.`); return; }
    if (artifacts.some((a) => a.target === target && a.file_type === fileType)) { setError('Файл для этой платформы и формата уже есть в релизе.'); return; }
    const pending = new AbortController(); controller.current = pending; setBusy(true); onBusy(true); setPercent(0);
    const data = new FormData(); data.append('target', target); if (format.trim()) data.append('file_type', format.trim()); data.append('file', file, file.name);
    try {
      await upload<Artifact>(artifactsPath(applicationId, releaseId), data, pending.signal, setPercent);
      if (mounted.current) { setFile(undefined); setNotice('Файл загружен и сохранён.'); }
    } catch (error) {
      if (mounted.current) {
        if (error instanceof DOMException && error.name === 'AbortError') setNotice('Загрузка отменена. Если сервер уже завершил сохранение, файл появится в списке.');
        else setError(releaseError(error));
      }
    } finally {
      controller.current = null;
      if (mounted.current) { setBusy(false); onBusy(false); onChange(); }
    }
  }
  const files: UploadFile[] = file ? [{ uid: 'selected', name: file.name, size: file.size, status: 'done' }] : [];
  return <>
    {error && <Alert className="form-alert" type="error" title={error} showIcon />}{notice && <Alert className="form-alert" type="info" title={notice} showIcon />}
    <Form layout="vertical" disabled={busy}>
      <Form.Item label="Целевая платформа" required><Select aria-label="Целевая платформа" value={target} onChange={setTarget} placeholder="Выберите платформу" options={options.targets.map((value) => ({ value, label: targetName(value) }))} /></Form.Item>
      <Form.Item label="Формат файла" extra={`Определяется по расширению. Для файла без расширения укажите явно. Например: ${options.file_types.join(', ')}.`}><Input aria-label="Формат файла" value={format} onChange={(e) => setFormat(e.target.value)} maxLength={32} placeholder="Автоматически" /></Form.Item>
      <Upload.Dragger disabled={busy} maxCount={1} fileList={files} beforeUpload={(selected) => {
        if (selected.size === 0 || selected.size > options.max_file_bytes) { setError(`Выберите непустой файл до ${size(options.max_file_bytes)}.`); return Upload.LIST_IGNORE; }
        setFile(selected); setError(''); setNotice(''); setPercent(0); return false;
      }} onRemove={() => { setFile(undefined); return true; }}>
        <p className="ant-upload-drag-icon"><InboxOutlined /></p><p>Перетащите установщик или выберите файл</p><Typography.Text type="secondary">До {size(options.max_file_bytes)} на файл</Typography.Text>
      </Upload.Dragger>
      {file && <Typography.Paragraph className="upload-file-summary">{file.name} · {size(file.size)} · {format.trim() || detectedType(file.name)?.toUpperCase() || 'Укажите формат'}</Typography.Paragraph>}
      {busy && <div className="upload-progress" role="status"><Progress percent={Math.floor(percent)} status="active" /><Typography.Text type="secondary">{percent >= 100 ? 'Передано. Сервер сохраняет и проверяет файл…' : 'Передача файла…'}</Typography.Text></div>}
      <Space wrap><Button type="primary" disabled={!file || !target || busy} loading={busy} onClick={() => { void send(); }}>Загрузить файл</Button>
        {busy && <Button disabled={false} danger onClick={() => controller.current?.abort()}>Отменить загрузку</Button>}</Space>
    </Form>
  </>;
}
