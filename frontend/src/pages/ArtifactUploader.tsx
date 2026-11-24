import { useEffect, useRef, useState } from 'react';
import { InboxOutlined } from '@ant-design/icons';
import { Alert, Button, Collapse, Form, Input, Progress, Result, Select, Space, Spin, Typography, Upload } from 'antd';
import type { UploadFile } from 'antd';
import { Link } from 'react-router-dom';
import { api, upload } from '../api/client';
import { artifactsPath, detectedType, releaseError, size, type Artifact, type ArtifactType, type UploadOptions } from '../api/releases';
import { missingStandaloneSettings, standalonePath, standaloneSettingsRoute, type StandaloneSettings } from '../api/standalone';

type UploadKind = ArtifactType | 'standalone';
type StandaloneCheck = {kind: 'loading'} | {kind: 'error'; message: string} | {kind: 'ready'; settings: StandaloneSettings};
const kinds: {value: UploadKind; role: ArtifactType; label: string; description: string}[] = [
  {value: 'installer', role: 'installer', label: 'Установочный файл', description: 'Устанавливает приложение: например, Setup EXE, MSI, DMG, DEB или RPM. Нужен один файл.'},
  {value: 'executable', role: 'executable', label: 'Исполняемый файл', description: 'Самостоятельное приложение, которое запускают напрямую. Нужен один файл, установщик и подпись не требуются.'},
  {value: 'archive', role: 'archive', label: 'Архив', description: 'ZIP, TAR.GZ или другой архив с приложением. Нужен один файл.'},
  {value: 'standalone', role: 'executable', label: 'Пакет обновления', description: 'Для обновления отдельного исполняемого файла. Нужны два файла: само приложение и его подпись издателя .jws.'},
  {value: 'updater', role: 'updater', label: 'Пакет обновления Tauri', description: 'Для приложения на Tauri. Нужны два файла из одной сборки: пакет обновления и его подпись .sig.'},
];
const osName = (os: string) => ({windows: 'Windows', linux: 'Linux', darwin: 'macOS (darwin)', any: 'Любая ОС'})[os] ?? os;
const tauriFormats = (os?: string) => os === 'windows' ? ['exe', 'msi'] : os === 'linux' ? ['appimage'] : os === 'darwin' ? ['tar.gz'] : [];
const tauriHint = (os?: string) => os === 'windows' ? 'Установщик Tauri .exe или .msi' : os === 'linux' ? 'Пакет Tauri .AppImage' : 'Пакет Tauri .app.tar.gz';

export default function ArtifactUploader({ applicationId, releaseId, artifacts, options, published = false, onChange, onBusy }: { applicationId: string; releaseId: string; artifacts: Artifact[]; options: UploadOptions; published?: boolean; onChange: () => void; onBusy: (busy: boolean) => void }) {
  const [kind, setKind] = useState<UploadKind>(); const [os, setOs] = useState<string>(); const [target, setTarget] = useState<string>();
  const [file, setFile] = useState<File>(); const [signature, setSignature] = useState<File>(); const [format, setFormat] = useState('');
  const [busy, setBusy] = useState(false); const [percent, setPercent] = useState(0); const [error, setError] = useState(''); const [notice, setNotice] = useState('');
  const [uploaded, setUploaded] = useState(false);
  const [standalone, setStandalone] = useState<StandaloneCheck>(); const [settingsAttempt, setSettingsAttempt] = useState(0); const [checkingSettings, setCheckingSettings] = useState(false);
  const controller = useRef<AbortController | null>(null); const mounted = useRef(true);
  const signatureInput = useRef<HTMLInputElement | null>(null);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; controller.current?.abort(); }; }, []);
  useEffect(() => {
    if (kind !== 'standalone') { setStandalone(undefined); return; }
    const pending = new AbortController(); setStandalone({kind: 'loading'});
    api<StandaloneSettings>(standalonePath(applicationId), 'GET', undefined, pending.signal).then(
      (settings) => { if (!pending.signal.aborted) setStandalone({kind: 'ready', settings}); },
      (error: unknown) => { if (!pending.signal.aborted) setStandalone({kind: 'error', message: releaseError(error)}); },
    );
    return () => pending.abort();
  }, [kind, applicationId, settingsAttempt]);
  const missingSettings = standalone?.kind === 'ready' ? missingStandaloneSettings(standalone.settings) : undefined;
  const settingsReady = kind !== 'standalone' || (standalone?.kind === 'ready' && !missingSettings);
  const selectedKind = kinds.find((item) => item.value === kind);
  const role = selectedKind?.role;
  const targets = options.targets.filter((value) => kind === 'standalone' ? value !== 'any' && value !== 'darwin-universal' : kind === 'updater' ? value !== 'any' : true)
    .map((value) => ({value, os: value.split('-')[0] ?? value}));
  const systems = [...new Set(targets.map((item) => item.os))];
  const signing = kind === 'standalone' ? {label: 'Подпись издателя (.jws)', extension: '.jws', limit: 4096, field: 'standalone_signature'}
    : kind === 'updater' ? {label: 'Подпись Tauri (.sig)', extension: '.sig', limit: 16384, field: 'signature'} : undefined;
  const detectedFormat = (name: string) => detectedType(name, role === 'executable' || role === 'installer' ? target : undefined) ?? 'bin';
  function clearSignature() { setSignature(undefined); if (signatureInput.current) signatureInput.current.value = ''; }
  function resetFiles() { setFile(undefined); clearSignature(); setFormat(''); setError(''); setNotice(''); setPercent(0); }
  async function send() {
    if (controller.current || !file || !target || !role || !settingsReady) return;
    const fileType = format.trim().toLowerCase() || detectedFormat(file.name);
    setError(''); setNotice('');
    if (file.size === 0 || file.size > options.max_file_bytes) { setError(`Выберите непустой файл до ${size(options.max_file_bytes)}.`); return; }
    if (artifacts.some((a) => a.target === target && (a.distribution_role ?? 'installer') === role && (a.file_type === fileType || role === 'updater'))) { setError('Файл с этой платформой, форматом и типом уже есть. Для платформы допустим только один пакет обновления Tauri.'); return; }
    if (signing && !signature) { setError(`Прикрепите файл ${signing.extension} из той же сборки, что и пакет обновления.`); return; }
    if (kind === 'standalone' && artifacts.some((a) => a.target === target && a.standalone_signature_present)) { setError('Подписанное обновление для этой ОС и архитектуры уже есть в релизе.'); return; }
    const pending = new AbortController(); controller.current = pending; setBusy(true); onBusy(true); setPercent(0);
    const data = new FormData(); data.append('target', target); data.append('distribution_role', role); if (format.trim()) data.append('file_type', format.trim()); data.append('file', file, file.name);
    if (signing && signature) data.append(signing.field, signature, signature.name);
    let attemptedUpload = false;
    try {
      if (kind === 'standalone') {
        setCheckingSettings(true);
        const settings = await api<StandaloneSettings>(standalonePath(applicationId), 'GET', undefined, pending.signal);
        if (!mounted.current || pending.signal.aborted) return;
        setStandalone({kind: 'ready', settings});
        if (missingStandaloneSettings(settings)) { resetFiles(); return; }
        setCheckingSettings(false);
      }
      attemptedUpload = true;
      await upload<Artifact>(artifactsPath(applicationId, releaseId), data, pending.signal, setPercent);
      if (mounted.current) { setFile(undefined); setSignature(undefined); setUploaded(true); }
    } catch (error) {
      if (mounted.current) {
        if (error instanceof DOMException && error.name === 'AbortError') setNotice('Загрузка отменена. Если сервер уже завершил сохранение, файл появится в списке.');
        else setError(releaseError(error));
      }
    } finally {
      controller.current = null;
      if (mounted.current) { setBusy(false); setCheckingSettings(false); onBusy(false); if (attemptedUpload) onChange(); }
    }
  }
  if (uploaded) return <Result status="success" title="Файл загружен и сохранён." />;
  const files: UploadFile[] = file ? [{ uid: 'selected', name: file.name, size: file.size, status: 'done' }] : [];
  return <>
    {published && <Alert className="form-alert" type="warning" showIcon title="Релиз опубликован. После добавления файл нельзя будет изменить или удалить." />}
    {error && <Alert className="form-alert" type="error" title={error} showIcon />}{notice && <Alert className="form-alert" type="info" title={notice} showIcon />}
    <Form layout="vertical" disabled={busy}>
      <Form.Item label="1. Что загружаем" required extra={selectedKind?.description ?? 'Выберите назначение файла — форма подскажет остальные шаги.'}>
        <Select aria-label="Тип артефакта" placeholder="Выберите назначение" value={kind} onChange={(value) => { setKind(value); setOs(undefined); setTarget(undefined); resetFiles(); }} options={kinds.filter((item) => !options.artifact_types || options.artifact_types.includes(item.role))} />
      </Form.Item>
      {kind === 'standalone' && (standalone?.kind === 'ready' && !missingSettings ? <Typography.Paragraph type="secondary">UUID продукта и публичный ключ настроены. Подпись и соответствие файлу сервер проверит при загрузке.</Typography.Paragraph>
        : standalone?.kind === 'error' ? <Alert className="form-alert" type="error" showIcon title="Не удалось проверить настройки подписанных обновлений" description={standalone.message} action={<Button onClick={() => setSettingsAttempt((value) => value + 1)}>Повторить проверку</Button>} />
        : missingSettings ? <Alert className="form-alert" type="warning" showIcon title="Подписанные обновления не настроены" description={<><Typography.Paragraph>{missingSettings} Сначала сохраните их в настройках приложения.</Typography.Paragraph><Space wrap><Link to={standaloneSettingsRoute(applicationId)}>Настроить подписанные обновления</Link><Button onClick={() => setSettingsAttempt((value) => value + 1)}>Проверить настройки снова</Button></Space></>} />
        : <div className="form-alert" role="status"><Spin size="small" /> Проверяем настройки подписанных обновлений…</div>)}
      {kind && settingsReady && <div className="artifact-platform-fields"><Form.Item label="2. Операционная система" required>
        <Select aria-label="Операционная система" placeholder="Выберите ОС" value={os} onChange={(value) => { setOs(value); setTarget(value === 'any' ? 'any' : undefined); resetFiles(); }} options={systems.map((value) => ({value, label: osName(value)}))} />
      </Form.Item>
      {os && os !== 'any' && <Form.Item label="3. Архитектура" required extra="Сопоставьте обозначение с ОС и архитектурой в имени файла сборки.">
        <Select key={os} aria-label="Архитектура" placeholder="Выберите обозначение сборки" value={target} onChange={(value) => { setTarget(value); resetFiles(); }} options={targets.filter((item) => item.os === os).map((item) => ({value: item.value, label: item.value}))} />
      </Form.Item>}</div>}
      {target && settingsReady && <>
        <Typography.Paragraph>Платформа: <Typography.Text code>{target}</Typography.Text>{target === 'any' && ' — без привязки к ОС и архитектуре.'}</Typography.Paragraph>
        <Typography.Title level={5}>4. {signing ? 'Два файла из одной сборки' : 'Один файл'}</Typography.Title>
        <Form.Item label={kind === 'updater' ? tauriHint(os) : kind === 'standalone' ? 'Исполняемый файл приложения' : 'Файл'} required>
          <Upload.Dragger disabled={busy} maxCount={1} fileList={files} beforeUpload={(selected) => {
            if (selected.size === 0 || selected.size > options.max_file_bytes) { setError(`Выберите непустой файл до ${size(options.max_file_bytes)}.`); return Upload.LIST_IGNORE; }
            if (kind === 'updater' && !tauriFormats(os).includes(detectedType(selected.name) ?? '')) { setError(`Для ${osName(os ?? '')} нужен ${tauriHint(os)} и соответствующая подпись .sig.`); return Upload.LIST_IGNORE; }
            setFile(selected); clearSignature(); setError(''); setNotice(''); setPercent(0); return false;
          }} onRemove={() => { setFile(undefined); clearSignature(); return true; }}>
            <p className="ant-upload-drag-icon"><InboxOutlined /></p><p>Перетащите файл или выберите его</p><Typography.Text type="secondary">До {size(options.max_file_bytes)} на файл</Typography.Text>
          </Upload.Dragger>
        </Form.Item>
        {signing && <Form.Item label={signing.label} required extra={`Сначала выберите пакет, затем его подпись ${signing.extension}. До ${size(signing.limit)}.`}>
          <input ref={signatureInput} aria-label={signing.label} type="file" accept={signing.extension} disabled={busy || !file} key={kind} style={{maxWidth: '100%'}} onChange={(event) => {
            const selected = event.target.files?.[0];
            if (selected && (!selected.name.toLowerCase().endsWith(signing.extension) || selected.size === 0 || selected.size > signing.limit)) { setError(`Выберите непустой файл ${signing.extension} до ${size(signing.limit)}.`); event.target.value = ''; setSignature(undefined); return; }
            setSignature(selected); setError('');
          }} />{signature && <Button type="link" onClick={clearSignature}>Убрать подпись</Button>}
        </Form.Item>}
        {file && <Typography.Paragraph className="upload-file-summary">{file.name} · {size(file.size)} · {(format.trim() || detectedFormat(file.name)).toUpperCase()}</Typography.Paragraph>}
        {kind !== 'updater' && <Collapse size="small" className="upload-format-options" items={[{key: 'format', label: 'Дополнительно: формат файла', children: <Form.Item label="Формат файла" extra={`Обычно определяется автоматически. Примеры: ${options.file_types.join(', ')}.`}><Input aria-label="Формат файла" value={format} onChange={(event) => setFormat(event.target.value)} maxLength={32} placeholder="Автоматически" /></Form.Item>}]} />}
        {busy && <div className="upload-progress" role="status">{!checkingSettings && <Progress percent={Math.floor(percent)} status="active" />}<Typography.Text type="secondary">{checkingSettings ? 'Проверяем настройки подписанных обновлений…' : percent >= 100 ? 'Передано. Сервер сохраняет и проверяет файл…' : 'Передача файла…'}</Typography.Text></div>}
        <Space wrap><Button type="primary" disabled={!file || busy} loading={busy} onClick={() => { void send(); }}>Загрузить файл</Button>
          {busy && <Button disabled={false} danger onClick={() => controller.current?.abort()}>Отменить загрузку</Button>}</Space>
      </>}
    </Form>
  </>;
}
