import { useEffect, useState } from 'react';
import { Alert, App, Button, Card, Form, Input, Space, Spin, Typography } from 'antd';
import { api, ApiError, errorMessage } from '../api/client';
import { missingStandaloneSettings, standalonePath, standalonePublicKey, validProductId, type StandaloneSettings as Settings } from '../api/standalone';

export default function StandaloneSettings({ applicationId }: { applicationId: string }) {
  const { message } = App.useApp();
  const [settings, setSettings] = useState<Settings>(); const [attempt, setAttempt] = useState(0); const [loadError, setLoadError] = useState('');
  const [product, setProduct] = useState(''); const [key, setKey] = useState(''); const [productError, setProductError] = useState(''); const [keyError, setKeyError] = useState('');
  const [saving, setSaving] = useState(false); const [saveError, setSaveError] = useState('');
  const path = standalonePath(applicationId);
  useEffect(() => {
    const controller = new AbortController(); setSettings(undefined); setLoadError('');
    api<Settings>(path, 'GET', undefined, controller.signal).then((data) => {
      if (!controller.signal.aborted) { setSettings(data); setProduct(data.product_id ?? ''); setKey(data.public_key ?? ''); setProductError(''); setKeyError(''); }
    }, (error: unknown) => { if (!controller.signal.aborted) setLoadError(errorMessage(error)); });
    return () => controller.abort();
  }, [path, attempt]);
  async function save() {
    if (!settings || saving) return;
    setProductError(''); setKeyError(''); setSaveError('');
    const productId = product.trim();
    let publicKey: string | null;
    let invalid = false;
    if (productId && !validProductId(productId)) { setProductError('Введите UUID в строчном формате 8-4-4-4-12, как в конфигурации сборки.'); invalid = true; }
    try { publicKey = standalonePublicKey(key); }
    catch (error) { setKeyError(errorMessage(error)); return; }
    if (invalid) return;
    const changes: {product_id?: string | null; public_key?: string | null} = {};
    if ((productId || null) !== settings.product_id) changes.product_id = productId || null;
    if (publicKey !== settings.public_key) changes.public_key = publicKey;
    if (!Object.keys(changes).length) { setKey(publicKey ?? ''); return; }
    setSaving(true);
    try {
      await api(path, 'PUT', changes);
      void message.success('Настройки подписанных обновлений сохранены'); setAttempt((value) => value + 1);
    } catch (error) {
      setSaveError(error instanceof ApiError && error.code === 'invalid_standalone' ? 'Сервер отклонил UUID продукта или публичный ключ. Проверьте UUID и корректность Ed25519-ключа.' : errorMessage(error));
    } finally { setSaving(false); }
  }
  const dirty = settings && (product.trim() !== (settings.product_id ?? '') || key.trim() !== (settings.public_key ?? ''));
  return <Card title="Подписанные обновления">
    <Typography.Paragraph>Настройте один раз для обновлений с подписью .jws. UUID определяет программу, а публичный ключ проверяет подпись издателя. Для обычного скачивания файлов эти настройки не нужны.</Typography.Paragraph>
    {!settings ? loadError ? <Alert type="error" showIcon title="Не удалось загрузить настройки подписанных обновлений" description={loadError} action={<Button onClick={() => setAttempt((value) => value + 1)}>Повторить</Button>} /> : <div role="status"><Spin /> Загружаем настройки подписанных обновлений…</div> : <>
      {saveError && <Alert className="form-alert" type="error" title={saveError} showIcon />}
      {missingStandaloneSettings(settings) && <Alert className="form-alert" type="info" title={missingStandaloneSettings(settings)} showIcon />}
      <Form layout="vertical" disabled={saving} onFinish={() => { void save(); }}>
        <Form.Item label="UUID продукта" validateStatus={productError ? 'error' : undefined} help={productError} extra={settings.product_id_locked ? 'UUID закреплён после первой подписанной загрузки. Его нельзя изменить или очистить, даже после удаления черновика.' : 'Возьмите product_id из конфигурации сборки. Для новой программы издатель создаёт UUID один раз и использует его для всех версий.'}>
          <Input aria-label="UUID продукта" value={product} readOnly={settings.product_id_locked} maxLength={36} autoComplete="off" spellCheck={false} onChange={(event) => { setProduct(event.target.value); setProductError(''); setSaveError(''); }} placeholder="xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx" />
        </Form.Item>
        <Form.Item label="Публичный ключ Ed25519" validateStatus={keyError ? 'error' : undefined} help={keyError} extra={settings.public_key_locked ? 'Ключ закреплён существующими подписанными файлами и доступен только для чтения.' : 'Возьмите публичный ключ с машины сборки: вставьте весь публичный JWK (.jwk) или его поле x. Приватный ключ остаётся у издателя.'}>
          <Input.TextArea aria-label="Публичный ключ Ed25519" value={key} readOnly={settings.public_key_locked} maxLength={4096} autoSize={{minRows: 2, maxRows: 6}} autoComplete="off" spellCheck={false} onChange={(event) => { setKey(event.target.value); setKeyError(''); setSaveError(''); }} placeholder="Публичный JWK или строка x" />
        </Form.Item>
        <Typography.Paragraph type="secondary">Эти же UUID и ключ должны быть в доверенной конфигурации приложения, которое получает обновления. Пустое незакреплённое поле удаляет настройку; подписанные загрузки будут недоступны.</Typography.Paragraph>
        <Space wrap><Button type="primary" htmlType="submit" loading={saving} disabled={!dirty}>Сохранить настройки обновлений</Button><Button disabled={saving} onClick={() => { setSaveError(''); setAttempt((value) => value + 1); }}>Обновить настройки</Button></Space>
      </Form>
    </>}
  </Card>;
}
