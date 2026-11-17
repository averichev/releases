import { useEffect, useRef, useState } from 'react';
import { Alert, App, Button, Card, Collapse, Descriptions, Form, Input, Result, Segmented, Select, Space, Spin, Steps, Tag, Typography } from 'antd';
import { Link, useSearchParams } from 'react-router-dom';
import { api, ApiError, errorMessage } from '../api/client';
import { applicationPath, type Application } from '../api/applications';
import { tauriPath, type TauriSettings } from '../api/tauri';
import type { TauriPreview } from '../api/publication';
import CodeBlock from '../integrations/CodeBlock';
import { button, capabilities, client, command, register, runtime, signer, type Manager } from '../integrations/tauri';

const titles = ['Установка Tauri Updater', 'Создание ключей подписи', 'Настройка tauri.conf.json', 'Проверка обновлений в приложении', 'Сборка подписанного дистрибутива', 'Загрузка и публикация релиза', 'Проверка работоспособности'];
const official = 'https://v2.tauri.app/plugin/updater/';
const testSteps = ['Соберите и установите клиент 1.0.0 с Updater и вашим публичным ключом.', 'Соберите подписанную версию 1.0.1 тем же приватным ключом.', 'Загрузите пакет в новый релиз 1.0.1 в Veruna Releases.', 'Прикрепите соответствующую .sig.', 'Проверьте готовность и опубликуйте 1.0.1 в stable.', 'Запустите установленную 1.0.0.', 'Выполните проверку обновлений.', 'Убедитесь, что клиент обнаруживает 1.0.1.', 'Подтвердите загрузку и установку.', 'После перезапуска проверьте, что версия стала 1.0.1. Повторная проверка не должна предлагать обновление.'];
function Verify({ children }: {children: React.ReactNode}) { return <Alert type="info" showIcon title="Как проверить" description={children} />; }

function Diagnostics({ application, settings }: {application: Application; settings: TauriSettings}) {
  const [result, setResult] = useState<TauriPreview>(); const [testedVersion, setTestedVersion] = useState(''); const [error, setError] = useState('');
  const [network, setNetwork] = useState<{ok: boolean; text: string}>(); const [busy, setBusy] = useState(false);
  const request = useRef<AbortController | null>(null);
  useEffect(() => () => request.current?.abort(), []);
  async function check(values: {os: string; arch: string; current_version: string}) {
    request.current?.abort(); const c = new AbortController(); request.current = c;
    setBusy(true); setResult(undefined); setError(''); setNetwork(undefined);
    try {
      const preview = await api<TauriPreview>(`${applicationPath(application.id)}/updater/tauri/preview`, 'POST',
        {channel: settings.channel, target: `${values.os}-${values.arch}`, current_version: values.current_version}, c.signal);
      if (c.signal.aborted) return;
      setResult(preview); setTestedVersion(values.current_version);
      const url = settings.endpoint.replace('{{target}}', encodeURIComponent(values.os)).replace('{{arch}}', encodeURIComponent(values.arch)).replace('{{current_version}}', encodeURIComponent(values.current_version));
      try {
        const response = await fetch(url, {credentials: 'omit', cache: 'no-store', signal: AbortSignal.any([c.signal, AbortSignal.timeout(15000)])});
        // A proxy returning index.html with HTTP 200 is not an updater response.
        const body: unknown = response.status === 200 ? await response.json() : null;
        const valid = response.status === 204 || (response.status === 200 && typeof body === 'object' && body !== null && 'version' in body && 'url' in body && 'signature' in body && body.version === preview.response?.version && body.url === preview.response?.url && body.signature === preview.response?.signature);
        const expected = preview.state === 'available' ? 200 : preview.state === 'no_update' ? 204 : null;
        setNetwork({ok: valid && response.status === expected, text: `Публичный endpoint: HTTP ${response.status}.${valid && response.status === expected ? ' Ответ соответствует preview.' : ' Ответ не соответствует preview; проверьте сервер, proxy и актуальность канала.'}`});
      } catch {
        if (!c.signal.aborted) setNetwork({ok: false, text: 'Серверный endpoint недоступен из браузера. Проверьте download.public_base_url, DNS, TLS, reverse proxy и CORS при другом origin.'});
      }
    } catch (e) { if (!c.signal.aborted) setError(e instanceof ApiError && e.code === 'invalid_release' ? 'Укажите корректный SemVer установленного клиента, например 1.0.0.' : errorMessage(e)); }
    finally { if (!c.signal.aborted) setBusy(false); }
  }
  return <>
    <Typography.Paragraph>Проверка серверного протокола использует существующий preview: явный текущий релиз канала, SemVer и совместимый updater-пакет. Затем браузер запрашивает публичный endpoint без административной сессии.</Typography.Paragraph>
    <Form layout="vertical" disabled={busy} initialValues={{os: 'windows', arch: 'x86_64', current_version: '1.0.0'}} onValuesChange={() => {setResult(undefined); setError(''); setNetwork(undefined);}} onFinish={(v) => {void check(v);}}>
      <div className="tauri-diagnostic-fields">
        <Form.Item label="Канал"><Input value={settings.channel} readOnly /></Form.Item>
        <Form.Item name="os" label="Операционная система" rules={[{required: true}]}><Select options={[{value: 'windows', label: 'Windows'}, {value: 'linux', label: 'Linux'}, {value: 'darwin', label: 'macOS (darwin)'}]} /></Form.Item>
        <Form.Item name="arch" label="Архитектура" rules={[{required: true}]}><Select options={['x86_64', 'aarch64', 'i686', 'armv7'].map((value) => ({value}))} /></Form.Item>
        <Form.Item name="current_version" label="Текущая установленная версия" rules={[{required: true, message: 'Укажите SemVer клиента'}]}><Input maxLength={128} /></Form.Item>
      </div><Button type="primary" htmlType="submit" loading={busy}>Проверить обновление</Button>
    </Form>
    {error && <Alert className="form-alert" type="error" showIcon title="Не удалось проверить обновление" description={error} />}
    {network && <Alert className="form-alert" showIcon type={network.ok ? 'info' : 'error'} title={network.ok ? 'Публичный endpoint отвечает' : 'Серверный endpoint недоступен или ответ не соответствует протоколу'} description={network.text} />}
    {result && <div aria-live="polite"><Result status={result.state === 'available' ? 'success' : result.state === 'blocked' ? 'warning' : 'info'} title={result.state === 'available' ? 'Сервер предлагает обновление' : result.state === 'blocked' ? 'Обновление заблокировано' : 'Обновления нет — протокол 204'} subTitle={result.message} />
      <Descriptions column={1} items={[
        {key: 'installed', label: 'Версия установленного клиента', children: testedVersion},
        {key: 'url', label: 'URL скачивания', children: result.response ? <Typography.Text copyable className="distribution-url">{result.response.url}</Typography.Text> : 'Не сформирован'},
        {key: 'version', label: 'Текущий релиз канала', children: result.selected_version ?? 'Не назначен'},
        {key: 'artifact', label: 'Артефакт', children: result.artifact_filename ?? result.artifact_id ?? 'Не выбран'},
        {key: 'signature', label: 'Подпись .sig', children: result.signature_present ? 'Сохранена, структура корректна; криптография здесь не проверяется' : 'Не подтверждена для выбранного пакета'},
        {key: 'reason', label: 'Причина выбора', children: `${result.message} Источник — указатель канала ${settings.channel}. (${result.reason})`},
      ]} />
      {result.response != null && <CodeBlock title="JSON-ответ Tauri" language="json" code={JSON.stringify(result.response, null, 2)} />}
    </div>}
    <Collapse items={[{key: 'curl', label: 'Проверка протокола вне браузера: curl', children: <><Typography.Paragraph>Пример для Windows x86_64 и установленной версии 1.0.0. Измените значения при проверке другой платформы. Команда показывает реальный HTTP-статус и ответ сервера; ограничения CORS браузера к ней не относятся.</Typography.Paragraph><CodeBlock title="Проверка Tauri endpoint" code={`curl -i '${settings.endpoint.replace('{{target}}', 'windows').replace('{{arch}}', 'x86_64').replace('{{current_version}}', '1.0.0')}'`} /></>}]} />
    <Alert className="form-alert" type="warning" showIcon title="Реальная установка проверяется в установленном Tauri-приложении" description="Серверная диагностика не доказывает проверку подписи, установку пакета и перезапуск. Результат установки клиента на сервере не сохраняется." />
    <Collapse items={[{key: 'scenario', label: 'Сценарий 1.0.0 → 1.0.1', children: <ol>{testSteps.map((v) => <li key={v}>{v}</li>)}</ol>}]} defaultActiveKey={['scenario']} />
    <Typography.Paragraph className="form-alert">Приложения, выпущенные без Tauri Updater, не смогут получить его автоматически. Сначала пользователь должен вручную установить сборку с интеграцией.</Typography.Paragraph>
    <Verify>Пройдите сценарий на каждой поддерживаемой ОС. Сравните реальную версию после перезапуска; равная или более новая версия не должна приводить к downgrade.</Verify>
  </>;
}

export default function TauriIntegration({ application, administrator }: {application: Application; administrator: boolean}) {
  const [params, setParams] = useSearchParams(); const { message } = App.useApp();
  const channel = params.get('channel') === 'beta' ? 'beta' : 'stable';
  const manager = (['npm', 'pnpm', 'yarn', 'bun'].includes(params.get('manager') ?? '') ? params.get('manager') : 'pnpm') as Manager;
  const number = Number(params.get('step') ?? 1); const step = Number.isInteger(number) && number >= 1 && number <= 7 ? number - 1 : 0;
  function choose(key: string, value: string) { setParams((old) => {const next = new URLSearchParams(old); next.set(key, value); return next;}, {replace: true}); }
  const [settings, setSettings] = useState<TauriSettings>(); const [error, setError] = useState(''); const [attempt, setAttempt] = useState(0);
  const [keyDraft, setKeyDraft] = useState(''); const [saving, setSaving] = useState(false); const [saveError, setSaveError] = useState(''); const [checking, setChecking] = useState(false);
  const base = tauriPath(application.id);
  useEffect(() => {
    const c = new AbortController(); setSettings(undefined); setError(''); setSaveError('');
    api<TauriSettings>(`${base}?channel=${channel}`, 'GET', undefined, c.signal).then((s) => {if (!c.signal.aborted) {setSettings(s); setKeyDraft(s.public_key ?? '');}}, (e) => {if (!c.signal.aborted) setError(errorMessage(e));});
    return () => c.abort();
  }, [base, channel, attempt]);
  async function save() {
    setSaving(true); setSaveError('');
    try { await api(base, 'PUT', {public_key: keyDraft.trim() || null}); void message.success('Публичный ключ сохранён'); setAttempt((v) => v + 1); }
    catch (e) {setSaveError(e instanceof ApiError && e.code === 'invalid_updater' ? 'Введите полное base64-содержимое публичного .key.pub. Приватные ключи, подписи и пути к файлам не принимаются.' : errorMessage(e));} finally {setSaving(false);}
  }
  async function readiness() {
    setChecking(true); setError('');
    try {setSettings(await api<TauriSettings>(`${base}/readiness?channel=${channel}`, 'POST', undefined, undefined, 0));}
    catch (e) {setError(errorMessage(e));} finally {setChecking(false);}
  }
  if (!settings) return error ? <Alert type="error" showIcon title="Не удалось загрузить интеграцию" description={error} action={<Button onClick={() => setAttempt((v) => v + 1)}>Повторить</Button>} /> : <Spin aria-label="Загрузка Tauri-интеграции" />;
  const releases = `/admin/applications/${encodeURIComponent(application.id)}/releases`;
  const content = [
    <Space orientation="vertical" className="full-width" key="install">
      <Typography.Paragraph>Плагин обновлений устанавливается в клиентское Tauri 2 приложение {application.name}. Выполните команды из корня его проекта, где настроен Tauri CLI.</Typography.Paragraph>
      <CodeBlock title="Установка плагинов" code={`${command(manager, 'add updater')}\n${command(manager, 'add process')}`} />
      <Typography.Paragraph>Updater проверяет и устанавливает пакеты. Process нужен JavaScript-коду для перезапуска. В src-tauri/src/lib.rs добавьте регистрацию плагинов в существующий setup Tauri Builder:</Typography.Paragraph>
      <CodeBlock title="Добавления в lib.rs" language="rust" code={register} />
      <Typography.Paragraph>Добавьте эти разрешения в массив permissions существующего capability вашего локального окна (обычно src-tauri/capabilities/default.json). Сохраните identifier, windows и остальные разрешения.</Typography.Paragraph>
      <CodeBlock title="Дополнение capabilities" language="json" code={capabilities} />
      <Alert type="warning" title="Не заменяйте целиком lib.rs и capabilities" description="Сохраните команды, плагины и setup вашего проекта. Не регистрируйте один плагин дважды. Не выдавайте updater-разрешения недоверенному удалённому контенту." showIcon />
      <Verify>В Cargo.toml есть tauri-plugin-updater и tauri-plugin-process, в package.json — их JS-пакеты; сборка проходит, capability относится к нужному окну.</Verify>
    </Space>,
    <Space orientation="vertical" className="full-width" key="keys">
      <Typography.Paragraph>Цифровая подпись защищает обновления от подмены. Для независимого приложения {application.name} создайте собственную пару ключей на машине разработчика или сборки. В Unix-оболочке сначала создайте каталог:</Typography.Paragraph>
      <CodeBlock title="Создание пары ключей" code={`mkdir -p ~/.tauri\n${signer(manager, application.slug)}`} />
      <Typography.Paragraph>В PowerShell предварительно создайте каталог командой New-Item -ItemType Directory -Force ~/.tauri. Команда signer работает через выбранный пакетный менеджер; в командной строке используйте путь домашнего каталога вашей оболочки.</Typography.Paragraph>
      <Alert type="warning" showIcon title="Приватный ключ остаётся у разработчика" description={`Файл ${application.slug}.key используется только при сборке. Сохраните резервную копию и пароль в защищённом хранилище; не отправляйте их Veruna Releases и не добавляйте в Git. Потеря ключа блокирует обновления уже установленных клиентов этим ключом.`} />
      <Typography.Paragraph>Файл {application.slug}.key.pub содержит публичный ключ. Он встраивается в Tauri-клиент и не является секретом. Сохраните полное base64-содержимое .key.pub ниже, чтобы получить готовую конфигурацию.</Typography.Paragraph>
      {administrator ? <Form layout="vertical" onFinish={() => {void save();}}>
        <Form.Item label="Публичный ключ Tauri (.key.pub)" validateStatus={saveError ? 'error' : undefined} help={saveError || 'Только публичный ключ, до 4096 символов. Пустое поле удаляет настройку.'}>
          <Input.TextArea aria-label="Публичный ключ Tauri" value={keyDraft} onChange={(e) => setKeyDraft(e.target.value)} rows={4} maxLength={4096} disabled={saving} autoComplete="off" spellCheck={false} />
        </Form.Item><Button htmlType="submit" type="primary" loading={saving}>Сохранить публичный ключ</Button>
      </Form> : <><Alert type="info" title="Сохранить публичный ключ может администратор" /><Typography.Paragraph copyable={!!settings.public_key} className="distribution-url">{settings.public_key ?? 'Ключ не указан'}</Typography.Paragraph></>}
      <Collapse items={[{key: 'rotation', label: 'Почему смена ключа требует осторожности', children: 'Сохранение другого публичного ключа меняет только подсказки Veruna. Установленные клиенты продолжают доверять встроенному ключу. Ротацию нужно заранее реализовать в клиенте; сервер не меняет его доверие удалённо.'}]} />
      <Verify>Сохраните содержимое .key.pub и откройте следующий шаг. Конфигурация должна содержать именно этот ключ. Сервер проверяет формат ключа, но не его соответствие пакетам.</Verify>
    </Space>,
    <Space orientation="vertical" className="full-width" key="config">
      <Typography.Paragraph>Объедините фрагмент с существующим src-tauri/tauri.conf.json. Сохраните productName, version, identifier и прочие параметры. Адрес ниже получен из серверного download.public_base_url.</Typography.Paragraph>
      {settings.configuration ? <CodeBlock title="Фрагмент tauri.conf.json" language="json" code={JSON.stringify(settings.configuration, null, 2)} /> : <Alert type="warning" showIcon title="Не настроен публичный ключ" description="Сохраните настоящий .key.pub на шаге 2 — после этого появится готовый JSON. Endpoint уже показан вверху страницы." />}
      <Alert type="info" showIcon title={`Канал сборки: ${channel}`} description="Переключатель Stable / Beta меняет endpoint примера. Статическая конфигурация фиксирует канал на этапе сборки; смена релиза на сервере не переключает установленный клиент между stable и beta. Runtime-вариант — в расширенной части шага 4." />
      <Verify>Проверьте JSON, публичный ключ, HTTPS и slug. Оставьте переменные {'{{target}}'}, {'{{arch}}'} и {'{{current_version}}'} без подстановки: их заполнит Tauri.</Verify>
    </Space>,
    <Space orientation="vertical" className="full-width" key="client">
      <Typography.Paragraph>Сохраните функцию в клиентском проекте, например в client.ts. Она показывает версию, запрашивает согласие, отображает прогресс и ошибки. Перед установкой предложите пользователю сохранить работу.</Typography.Paragraph>
      <CodeBlock title="Функция проверки и установки" language="typescript" code={client} />
      <Alert type="warning" showIcon title="Перезапуск зависит от ОС" description="Windows запускает установщик и завершает приложение; строки после установки могут не выполниться, перезапуск контролирует установщик. Linux и macOS требуют relaunch() для запуска новой версии. Поведение установщика проверяйте на каждой целевой ОС." />
      <Collapse items={[
        {key: 'button', label: 'Проверка вручную: Настройки → Проверить обновления', children: <CodeBlock title="Подключение к кнопке" language="typescript" code={button} />},
        {key: 'auto', label: 'Автоматическая проверка при запуске или по расписанию', children: <><Typography.Paragraph>Вызывайте эту же функцию после запуска интерфейса или с выбранным интервалом. Она запрашивает согласие перед загрузкой и установкой. Для фоновой проверки без диалога отдельно вызовите check(), покажите уведомление о версии и освободите Update через close(), если не устанавливаете. Не запускайте параллельные проверки и не устанавливайте без уведомления.</Typography.Paragraph><CodeBlock title="Проверка при запуске" language="typescript" code={`// После инициализации интерфейса; согласие на установку остаётся обязательным.\nvoid checkForUpdates((message) => {\n  const status = document.querySelector('#update-status');\n  if (status) status.textContent = message;\n});`} /></>},
        {key: 'runtime', label: 'Расширенно: stable / beta во время работы клиента', children: <><Typography.Paragraph>Runtime endpoints задаются через Rust API. Пример использует два разрешённых адреса именно этого приложения и возвращает Update выбранного канала. Хранение выбора и вызов из интерфейса реализуются в клиенте. Veruna не изменяет настройки установленного приложения.</Typography.Paragraph><CodeBlock title="Runtime endpoints через Rust" language="rust" code={runtime(settings.endpoints)} /><Typography.Paragraph>После согласия пользователя используйте возвращённый Update для download_and_install, обрабатывайте прогресс и ошибки, затем app.restart() на Linux/macOS. Не вызывайте после этого JS check() с прежней статической конфигурацией: он снова проверит канал сборки. Возврат в stable не выполняет downgrade.</Typography.Paragraph><Typography.Link href="https://docs.rs/tauri-plugin-updater/latest/tauri_plugin_updater/struct.UpdaterBuilder.html" target="_blank" rel="noreferrer">Rust UpdaterBuilder API</Typography.Link></>},
      ]} />
      <Verify>Проверьте отсутствие обновлений, отказ от установки, ошибки сети и прогресс. Окончательный тест запускайте из установленной сборки, не только из tauri dev.</Verify>
    </Space>,
    <Space orientation="vertical" className="full-width" key="build">
      <Typography.Paragraph>bundle.createUpdaterArtifacts = true включает создание updater-пакетов и .sig. Задайте приватный ключ и пароль только в окружении сборки. Tauri не загружает эти переменные из .env автоматически.</Typography.Paragraph>
      <CodeBlock title="Сборка в Bash / zsh" code={`export TAURI_SIGNING_PRIVATE_KEY="$HOME/.tauri/${application.slug}.key"\n# Введите пароль без вывода на экран и без сохранения в истории команд:\nprintf 'Пароль ключа: '\nIFS= read -r -s TAURI_SIGNING_PRIVATE_KEY_PASSWORD\nprintf '\\n'\nexport TAURI_SIGNING_PRIVATE_KEY_PASSWORD\n${command(manager, 'build')}\nunset TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PASSWORD`} />
      <Collapse items={[{key: 'powershell', label: 'Сборка в PowerShell', children: <CodeBlock title="Переменные PowerShell" code={`$env:TAURI_SIGNING_PRIVATE_KEY = "$HOME/.tauri/${application.slug}.key"\n$secret = Read-Host "Пароль ключа" -AsSecureString\n$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = [System.Net.NetworkCredential]::new("", $secret).Password\ntry { ${command(manager, 'build')} }\nfinally {\n  Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY\n  Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD\n}`} />}]} />
      <ul><li>Windows: NSIS *.exe или *.msi и соответствующая .sig.</li><li>Linux: *.AppImage и *.AppImage.sig.</li><li>macOS: *.app.tar.gz и *.app.tar.gz.sig из bundle/macos.</li></ul>
      <Alert type="warning" showIcon title="Обычный установщик может не подходить updater" description="macOS DMG и Linux DEB/RPM не являются updater-пакетами этого адаптера. Произвольный EXE тоже не гарантирует совместимость: нужен установщик Tauri. Имена и набор файлов зависят от bundler и версии Tauri. Режим v1Compatible здесь не используется." />
      <Typography.Paragraph>Подпись Tauri Updater проверяет обновление ключом клиента. Платформенный code signing Windows, macOS и notarization — отдельные настройки сборки; подпись updater их не заменяет.</Typography.Paragraph>
      <Verify>Найдите updater-пакет и его .sig в target/release/bundle. Убедитесь, что version клиента соответствует будущему SemVer релиза. Приватный ключ и пароль не должны попасть в Git или артефакты релиза.</Verify>
    </Space>,
    <Space orientation="vertical" className="full-width" key="publish">
      <Typography.Paragraph>Используйте существующие релизы и каналы {application.name}. Подпись загружается отдельно для точного пакета.</Typography.Paragraph>
      <ol><li>Откройте <Link to={releases}>Релизы приложения {application.name}</Link> и создайте новый релиз.</li><li>Укажите SemVer, совпадающий с версией Tauri-приложения, и описание изменений.</li><li>Загрузите updater-пакет и выберите точную платформу, например windows-x86_64.</li><li>Назначьте файлу роль «Tauri updater» и прикрепите соответствующий .sig.</li><li>Повторите для каждой поддерживаемой платформы.</li><li>Выполните существующую проверку готовности релиза, исправьте ошибки и предупреждения подписи.</li><li>Опубликуйте в нужном канале ({channel}) и убедитесь, что релиз назначен текущим.</li></ol>
      <CodeBlock title="Публичный релиз и файлы после публикации" code={`curl --fail-with-body '${settings.public_base_url}/api/v1/public/applications/${application.slug}?channel=${channel}'`} />
      <Descriptions column={1} items={[
        {key: 'published', label: 'Опубликованный релиз', children: settings.published_release_exists ? 'Есть' : 'Нет'},
        {key: 'current', label: `Текущий релиз ${channel}`, children: settings.current_version ?? 'Не назначен'},
        {key: 'url', label: 'Публичный адрес', children: settings.public_base_url},
      ]} />
      {settings.packages.length ? <ul>{settings.packages.map((p) => <li key={p.id}><strong>{p.target}</strong> — {p.filename}: {p.compatible ? 'совместимый формат' : 'несовместимый формат'}, {p.signature_present ? '.sig сохранена' : '.sig отсутствует или некорректна'}</li>)}</ul> : <Alert type="warning" title="В текущем релизе канала нет updater-пакетов" />}
      <Button loading={checking} disabled={saving} onClick={() => {void readiness();}}>Проверить готовность и SHA-256 файлов</Button>
      {settings.full_integrity_checked && <Typography.Text>Полная проверка файлов выполнена. Результат отражает состояние на момент запроса.</Typography.Text>}
      {settings.validation && <Collapse items={[{key: 'validation', label: 'Результат существующей проверки публикации', children: <><Typography.Paragraph>Состояние: {settings.validation.state}</Typography.Paragraph>{[...settings.validation.errors, ...settings.validation.warnings].map((i, index) => <Alert key={index} type="warning" title={i.message} />)}{settings.validation.state === 'ready' && <Typography.Paragraph>Проверки публикации пройдены.</Typography.Paragraph>}</>}]} />}
      <Alert type="warning" showIcon title="Наличие .sig не доказывает подлинность" description="Сервер проверяет структуру подписи, а не криптографическое соответствие пакета публичному ключу. Это проверит Tauri-клиент. Опубликованные файлы и подписи неизменяемы — исправление требует нового релиза." />
      <Verify>В текущем релизе выбранного канала есть ровно один совместимый подписанный updater-пакет на каждую нужную платформу. Затем выполните шаг 7.</Verify>
    </Space>,
    <Diagnostics key={channel} application={application} settings={settings} />,
  ][step];
  return <Space orientation="vertical" className="full-width tauri-integration">
    <Card title="Tauri Updater" extra={<Button disabled={saving || checking} onClick={() => setAttempt((v) => v + 1)}>Обновить данные</Button>}>
      <Space wrap className="release-toolbar"><Segmented aria-label="Канал обновлений" value={channel} disabled={saving || checking} onChange={(v) => choose('channel', String(v))} options={[{value: 'stable', label: 'Stable'}, {value: 'beta', label: 'Beta'}]} />
        <Select aria-label="Пакетный менеджер" value={manager} onChange={(v) => choose('manager', v)} options={['npm', 'pnpm', 'yarn', 'bun'].map((value) => ({value}))} />
        <Tag color={settings.ready ? 'green' : 'gold'}>{settings.ready ? 'Серверная конфигурация готова' : 'Требуется настройка'}</Tag>
      </Space>
      <Descriptions column={1} size="small" items={[
        {key: 'app', label: 'Приложение / slug', children: `${application.name} / ${application.slug}`},
        {key: 'version', label: `Текущий релиз ${channel}`, children: settings.current_version ?? 'Не назначен'},
        {key: 'endpoint', label: 'URL сервера обновлений', children: <Typography.Text copyable className="distribution-url">{settings.endpoint}</Typography.Text>},
      ]} />
      {settings.blockers.length > 0 && <Alert className="form-alert" type="warning" showIcon title="Условия готовности не выполнены" description={<ul>{settings.blockers.map((i, index) => <li key={index}>{i.message}</li>)}</ul>} />}
      <Typography.Paragraph type="secondary" className="form-alert">Готовность относится к серверной конфигурации и доступным платформам. Она не подтверждает внешнюю доступность endpoint и реальную установку клиента — проверьте шаг 7.</Typography.Paragraph>
      <Typography.Link href={official} target="_blank" rel="noreferrer">Официальная документация Tauri 2 Updater</Typography.Link>
    </Card>
    {error && <Alert type="error" showIcon title={error} />}
    <div className="tauri-wizard"><Card className="tauri-step-navigation"><Steps orientation="vertical" current={step} onChange={(v) => choose('step', String(v + 1))} items={titles.map((title, i) => ({title, status: i === step ? 'process' : 'wait'}))} /></Card>
      <Card className="tauri-step-content" title={`${step + 1}. ${titles[step]}`}>
        {content}
        <Space className="tauri-step-buttons" wrap><Button disabled={step === 0} onClick={() => choose('step', String(step))}>Назад</Button><Button type="primary" disabled={step === 6} onClick={() => choose('step', String(step + 2))}>Далее</Button><Typography.Text type="secondary">Шаги можно открывать в любом порядке</Typography.Text></Space>
      </Card>
    </div>
  </Space>;
}
