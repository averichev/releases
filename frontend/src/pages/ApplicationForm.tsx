import { useEffect, useRef, useState, type ReactNode } from 'react';
import { Alert, Avatar, Button, Form, Input, Radio, Space, Tag, Typography } from 'antd';
import { AppstoreOutlined, UploadOutlined } from '@ant-design/icons';
import { suggestSlug, validSlug, type Application, type ApplicationValues } from '../api/applications';

export function ApplicationTags({ application }: { application: Application }) {
  return <Space size={4} wrap><Tag color={application.status === 'active' ? 'green' : 'default'}>{application.status === 'active' ? 'Активное' : 'В архиве'}</Tag><Tag>{application.visibility === 'public' ? 'Публичное' : 'Приватное'}</Tag></Space>;
}
export function ApplicationIcon({ application, size = 48 }: { application: Application; size?: number }) {
  return <Avatar shape="square" size={size} src={application.icon_url ?? undefined} icon={<AppstoreOutlined />} className="application-icon" />;
}
export function IconPicker({ file, onChange, disabled }: { file: File | null; onChange: (file: File | null) => void; disabled: boolean }) {
  const [error, setError] = useState('');
  const [preview, setPreview] = useState<string>();
  useEffect(() => { if (!file) { setPreview(undefined); return; } const url = URL.createObjectURL(file); setPreview(url); return () => URL.revokeObjectURL(url); }, [file]);
  return <div className="icon-picker">
    <Space wrap>{preview && <Avatar shape="square" size={56} src={preview} />}
      <label className={`icon-file-label${disabled ? ' disabled' : ''}`}>
        <UploadOutlined /> {file ? 'Выбрать другую иконку' : 'Выбрать иконку'}
        <input type="file" aria-label="Файл иконки" accept="image/svg+xml,.svg,image/png,image/jpeg,image/webp" disabled={disabled} onChange={(e) => {
          const selected = e.target.files?.[0]; e.target.value = '';
          if (!selected) return;
          const supported = ['image/svg+xml', 'image/png', 'image/jpeg', 'image/webp'].includes(selected.type) || /\.svg$/i.test(selected.name);
          if (selected.size > 2 * 1024 * 1024 || !supported) { setError('Выберите SVG, PNG, JPEG или WebP до 2 МиБ.'); return; }
          setError(''); onChange(selected);
        }} />
      </label>
      {file && <Button disabled={disabled} onClick={() => { setError(''); onChange(null); }}>Отменить выбор</Button>}
    </Space>
    <Typography.Paragraph type="secondary" className="icon-help">SVG, PNG, JPEG или WebP · до 2 МиБ · растровые до 1024×1024 пикселей</Typography.Paragraph>
    {error && <Alert type="error" title={error} showIcon />}
  </div>;
}
export default function ApplicationForm({ application, busy, onSubmit, submitText, children }: {
  application?: Application; busy: boolean; onSubmit: (values: ApplicationValues) => void; submitText: string; children?: ReactNode;
}) {
  const [form] = Form.useForm<ApplicationValues>();
  const manualSlug = useRef(false);
  return <Form form={form} layout="vertical" requiredMark={false} disabled={busy}
    initialValues={application ?? { name: '', slug: '', description: '', visibility: 'private' }}
    onValuesChange={(changed: Partial<ApplicationValues>) => {
      if ('slug' in changed) manualSlug.current = true;
      if (!application && changed.name !== undefined && !manualSlug.current) form.setFieldValue('slug', suggestSlug(changed.name));
    }} onFinish={onSubmit}>
    <Form.Item label="Название" name="name" rules={[{ required: true, whitespace: true, message: 'Введите название приложения.' }, { validator: async (_, value: string) => {
      if (value && ([...value.trim()].length > 120 || /[\u0000-\u001f\u007f-\u009f]/.test(value))) throw new Error('До 120 символов, без управляющих символов.');
    } }]}><Input autoComplete="off" placeholder="Название приложения" /></Form.Item>
    <Form.Item label="Slug" name="slug" extra={application ? 'Постоянный адрес. После создания slug не изменяется.' : 'Латинские буквы a-z, цифры и дефисы, до 64 символов. После создания изменить нельзя.'}
      rules={[{ required: true, message: 'Укажите slug.' }, { validator: async (_, value: string) => { if (value && !validSlug(value)) throw new Error('Недопустимый или служебный slug. Дефис не может стоять в начале или конце.'); } }]}>
      <Input readOnly={!!application} maxLength={64} autoComplete="off" placeholder="prosto-crm" />
    </Form.Item>
    <Form.Item label="Описание" name="description" rules={[{ validator: async (_, value: string) => { if (value && ([...value.trim()].length > 2000 || /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]/.test(value))) throw new Error('До 2000 символов, без управляющих символов.'); } }]}>
      <Input.TextArea autoSize={{ minRows: 4, maxRows: 10 }} placeholder="Для чего предназначено приложение" />
    </Form.Item>
    <Form.Item label="Публичность" name="visibility" extra="Публичность определяет будущую доступность продукта. Публичные страницы будут доступны позднее.">
      <Radio.Group options={[{ value: 'private', label: 'Приватное' }, { value: 'public', label: 'Публичное' }]} />
    </Form.Item>
    {children}
    <Button type="primary" htmlType="submit" loading={busy}>{submitText}</Button>
  </Form>;
}
