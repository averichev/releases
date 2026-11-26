import { useEffect, useState } from 'react';
import { Alert, App, Button, Card, Descriptions, Progress, Result, Space, Spin, Typography } from 'antd';
import { api, ApiError, errorMessage } from '../api/client';
import { useAuth } from '../auth/Auth';

const path = '/admin/self-update';
const stages = {
  idle: 'Проверка ещё не выполнялась', checking: 'Проверяем выпуск', available: 'Доступен подписанный выпуск',
  downloading: 'Загружаем файл', verifying: 'Проверяем файл', prepared: 'Выпуск готов к установке',
  stopping: 'Завершаем текущие запросы', switching: 'Заменяем исполняемый файл', starting: 'Запускаем сервис',
  health_check: 'Проверяем готовность сервиса', completed: 'Обновление установлено',
  rolling_back: 'Восстанавливаем предыдущую версию', rolled_back: 'Предыдущая версия восстановлена',
  failed: 'Операция завершилась с ошибкой',
};
type Status = {
  operation_id: string | null; state: keyof typeof stages; check_outcome: string; last_checked: number | null;
  available_version: string | null; downloaded_bytes: number; total_bytes: number | null;
  message: string; recovery_required: boolean;
};
type Summary = {
  current_version: string; target: string; mode: 'external' | 'standalone' | 'supervisor' | null;
  installation_supported: boolean; installation_reason: string | null; status: Status | null;
};
type Action = 'check' | 'prepare' | 'install' | 'cancel';
const canCheck = (status: Status) => ['idle', 'available', 'completed', 'rolled_back', 'failed'].includes(status.state) && !status.recovery_required;

function UpdatePanel() {
  const { modal } = App.useApp();
  const [summary, setSummary] = useState<Summary>();
  const [readError, setReadError] = useState('');
  const [actionError, setActionError] = useState<{ message: string; status: Status } | null>(null);
  const [request, setRequest] = useState<Action | null>(null);
  const [acceptedInstall, setAcceptedInstall] = useState<string | null>(null);
  const [revision, setRevision] = useState(0);

  useEffect(() => {
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    async function read() {
      let delay = 2000;
      try {
        const value = await api<Summary>(path, 'GET', undefined, controller.signal);
        if (controller.signal.aborted) return;
        setSummary(value);
        setReadError('');
        if (!value.status) return;
        if (canCheck(value.status)) delay = 10000;
      } catch (error) {
        if (controller.signal.aborted) return;
        setReadError(errorMessage(error));
        if (error instanceof ApiError && [401, 403].includes(error.status)) return;
      }
      timer = setTimeout(() => { void read(); }, delay);
    }
    void read();
    return () => { controller.abort(); clearTimeout(timer); };
  }, [revision]);

  async function act(action: Action, body?: object) {
    setRequest(action);
    setActionError(null);
    try {
      const result = await api<Status>(`${path}/${action}`, 'POST', body);
      if (action === 'install') setAcceptedInstall(result.operation_id);
      setSummary(previous => previous && { ...previous, status: result });
    } catch (error) {
      if (summary?.status) setActionError({ message: errorMessage(error), status: summary.status });
    } finally {
      setRequest(null);
      setRevision(value => value + 1);
    }
  }

  const status = summary?.status;
  const id = status?.operation_id;
  const version = status?.available_version;
  const disabled = request !== null || Boolean(readError);
  const installing = status?.state === 'prepared' && id === acceptedInstall;
  const cancelled = status?.state === 'failed' && status.message === 'Prepared update cancelled; check again';
  let title = status && stages[status.state];
  if (status?.state === 'idle' && status.check_outcome === 'no_update') title = 'Новых подписанных версий нет';
  if (installing) title = 'Установка принята, ожидаем перезапуска';
  if (cancelled) title = 'Подготовка отменена';
  return <>
    <div className="page-heading"><Typography.Title level={1}>Обновление сервиса</Typography.Title></div>
    <Space orientation="vertical" size="middle" style={{ width: '100%' }}>
      {readError && <Alert type="warning" showIcon title="Не удалось получить состояние сервиса"
        description={readError} action={<Button onClick={() => setRevision(value => value + 1)}>Повторить</Button>} />}
      {!summary && !readError && <Spin aria-label="Загрузка состояния обновлений" />}
      {summary && <Card title="Veruna Releases"><Descriptions column={1} items={[
        { key: 'version', label: 'Текущая версия', children: summary.current_version },
        { key: 'target', label: 'Платформа', children: summary.target },
        ...(status?.last_checked ? [{ key: 'checked', label: 'Последняя проверка', children: new Date(status.last_checked * 1000).toLocaleString('ru-RU') }] : []),
      ]} /></Card>}
      {summary && !summary.mode && <Alert type="info" showIcon title="Источник обновлений не настроен"
        description={<>Укажите адрес манифеста в <code>self_update.manifest_url</code> в конфигурации сервера и перезапустите Veruna Releases.</>} />}
      {status && <Card title={title} aria-live="polite">
        {version && <Typography.Paragraph>Версия выпуска <strong>{version}</strong></Typography.Paragraph>}
        {status.state === 'downloading' && status.total_bytes && <Progress percent={Math.floor(100 * status.downloaded_bytes / status.total_bytes)} />}
        {status.state === 'failed' && !cancelled && <Typography.Paragraph type="danger">{status.message}</Typography.Paragraph>}
        {status.recovery_required && <Alert type="error" title="Требуется восстановление на сервере" showIcon />}
        {summary.installation_reason && <Typography.Paragraph type="secondary">{summary.installation_reason}</Typography.Paragraph>}
        {actionError && actionError.status.operation_id === id && actionError.status.state === status.state &&
          <Alert type="error" showIcon title={actionError.message} />}
        <Space wrap>
          {canCheck(status) && <Button disabled={disabled} loading={request === 'check'} onClick={() => void act('check')}>Проверить обновления</Button>}
          {summary.installation_supported && status.state === 'available' && id && version &&
            <Button type="primary" disabled={disabled} loading={request === 'prepare'}
              onClick={() => void act('prepare', { check_id: id, version })}>Подготовить {version}</Button>}
          {summary.installation_supported && status.state === 'prepared' && id && !installing && <>
            <Button type="primary" disabled={disabled} loading={request === 'install'} onClick={() => modal.confirm({
              title: `Установить версию ${version}?`, content: 'Сервис завершит текущие запросы и перезапустится.',
              okText: 'Установить', cancelText: 'Отмена', onOk: () => act('install', { operation_id: id }),
            })}>Установить и перезапустить</Button>
            <Button disabled={disabled} onClick={() => void act('cancel', { operation_id: id })}>Отменить подготовку</Button>
          </>}
          {status.state === 'completed' && <Button type="primary" onClick={() => window.location.reload()}>Обновить страницу</Button>}
        </Space>
      </Card>}
    </Space>
  </>;
}

export default function ServiceUpdate() {
  const { state } = useAuth();
  return state.kind === 'ready' && state.user.role === 'admin'
    ? <UpdatePanel /> : <Result status="403" title="Требуются права администратора" />;
}
