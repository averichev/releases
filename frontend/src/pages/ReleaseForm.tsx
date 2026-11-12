import { Button, Form, Input } from 'antd';
import { validVersion, type Release, type ReleaseValues } from '../api/releases';
export default function ReleaseForm({ release, busy, onSubmit }: { release?: Release; busy: boolean; onSubmit: (values: ReleaseValues) => void }) {
  return <Form layout="vertical" initialValues={{ version: release?.version ?? '', title: release?.title ?? '', release_notes: release?.release_notes ?? '' }} onFinish={onSubmit} disabled={busy}>
    <Form.Item name="version" label="Версия" extra="SemVer: 1.2.0, 2.0.0-beta.1. Build metadata не создаёт отдельную версию." rules={[{ required: true, message: 'Укажите версию' }, { validator: (_, value: string) => validVersion(value ?? '') ? Promise.resolve() : Promise.reject(new Error('Введите корректную версию SemVer')) }]}><Input placeholder="1.2.0" maxLength={201} /></Form.Item>
    <Form.Item name="title" label="Название релиза" rules={[{ max: 120, message: 'Не более 120 символов' }]}><Input placeholder="Необязательно" maxLength={120} /></Form.Item>
    <Form.Item name="release_notes" label="Описание изменений" rules={[{ max: 20000, message: 'Не более 20 000 символов' }]}><Input.TextArea rows={7} maxLength={20000} showCount /></Form.Item>
    <Button type="primary" htmlType="submit" loading={busy}>{release ? 'Сохранить релиз' : 'Создать черновик'}</Button>
  </Form>;
}
