import { useState } from 'react';
import { Alert, Card, Form, Result, Typography } from 'antd';
import { Link, useNavigate } from 'react-router-dom';
import { useAuth } from '../auth/Auth';
import { api } from '../api/client';
import { applicationError, saveIcon, type Application, type ApplicationValues } from '../api/applications';
import ApplicationForm, { IconPicker } from './ApplicationForm';

export default function ApplicationCreate() {
  const { state } = useAuth(); const navigate = useNavigate();
  const [busy, setBusy] = useState(false); const [error, setError] = useState(''); const [file, setFile] = useState<File | null>(null);
  if (state.kind !== 'ready' || state.user.role !== 'admin') return <Result status="403" title="Доступ ограничен" subTitle="Создавать приложения может администратор." />;
  async function create(values: ApplicationValues) {
    setBusy(true); setError('');
    let created: Application;
    try { created = await api<Application>('/admin/applications', 'POST', values); }
    catch (error) { setError(applicationError(error)); setBusy(false); return; }
    if (file) {
      try { await saveIcon(created.id, file); }
      catch (error) { navigate(`/admin/applications/${created.id}/settings`, { replace: true, state: { iconError: `Приложение создано, но иконка не сохранена. ${applicationError(error)}` } }); return; }
    }
    navigate(`/admin/applications/${created.id}`, { replace: true });
  }
  return <>
    <Link className="back-link" to="/admin/applications">← Все приложения</Link>
    <div className="page-heading"><div className="eyebrow">КАТАЛОГ</div><Typography.Title level={1}>Создать приложение</Typography.Title><Typography.Paragraph>Название можно изменить позже. Slug закрепляется за приложением навсегда.</Typography.Paragraph></div>
    <Card className="application-form-card">
      {error && <Alert className="form-alert" type="error" showIcon title={error} />}
      <ApplicationForm busy={busy} onSubmit={(values) => { void create(values); }} submitText="Создать приложение">
        <Form.Item label="Иконка"><IconPicker file={file} onChange={setFile} disabled={busy} /></Form.Item>
      </ApplicationForm>
    </Card>
  </>;
}
