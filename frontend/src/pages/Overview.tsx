import { useEffect, useState } from 'react';
import { Alert, Badge, Button, Card, Tag, Typography } from 'antd';
import { ArrowRightOutlined, CheckCircleOutlined, ReloadOutlined, SafetyCertificateOutlined } from '@ant-design/icons';
import { Link } from 'react-router-dom';
import { getHealth, type Health } from '../api/health';
import { sections } from '../sections';

const { Title, Paragraph, Text } = Typography;

type HealthState = { kind: 'loading' } | { kind: 'ready'; data: Health } | { kind: 'error'; message: string };

function HealthCard() {
  const [state, setState] = useState<HealthState>({ kind: 'loading' });
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    const controller = new AbortController();
    getHealth(controller.signal).then(
      (data) => { if (!controller.signal.aborted) setState({ kind: 'ready', data }); },
      (error: unknown) => {
        if (!controller.signal.aborted) setState({ kind: 'error', message: error instanceof Error ? error.message : 'Не удалось проверить сервис' });
      },
    );
    return () => controller.abort();
  }, [attempt]);

  return (
    <Card className="health-card" title="Состояние сервиса" extra={
      <Button type="text" icon={<ReloadOutlined />} aria-label="Обновить состояние сервиса" loading={state.kind === 'loading'}
        onClick={() => { setState({ kind: 'loading' }); setAttempt((value) => value + 1); }} />
    }>
      <div role="status" aria-live="polite">
        <Badge status={state.kind === 'ready' ? 'success' : state.kind === 'error' ? 'error' : 'processing'}
          text={state.kind === 'ready' ? 'Сервис готов к работе' : state.kind === 'error' ? 'Сервис недоступен' : 'Проверяем соединение…'} />
        <Paragraph className="health-description">
          {state.kind === 'ready' ? 'HTTP-сервер отвечает, подключение к базе данных доступно.' : state.kind === 'error' ? state.message : 'Получаем актуальное состояние сервера.'}
        </Paragraph>
      </div>
      <div className="health-details"><Text type="secondary">Версия сервиса</Text><Text>{state.kind === 'ready' ? state.data.version : '—'}</Text></div>
      <div className="health-details"><Text type="secondary">Проверка доступности</Text><a href="/api/v1/health">/api/v1/health</a></div>
    </Card>
  );
}

export default function Overview() {
  return (
    <>
      <div className="page-heading"><div className="eyebrow">VERUNA RELEASES</div>
        <Title level={1}>Центр распространения</Title>
        <Paragraph>Единое место для приложений, релизов и доставки обновлений.</Paragraph>
      </div>
      <div className="overview-grid">
        <Card className="intro-card">
          <Tag color="green" variant="filled"><CheckCircleOutlined /> Каталог приложений</Tag>
          <Title level={2}>Каждому продукту —<br />постоянный адрес.</Title>
          <Paragraph>Создавайте и публикуйте релизы, переключайте каналы обновлений и проверяйте манифесты. Управление доступно в карточке каждого приложения.</Paragraph>
          <Link className="intro-link" to="/admin/applications">Перейти к приложениям <ArrowRightOutlined /></Link>
        </Card>
        <HealthCard />
      </div>
      <Alert className="stage-notice" type="info" showIcon icon={<SafetyCertificateOutlined />}
        title="Доступ к панели защищён"
        description="Операторы просматривают каталог, релизы и скачивают файлы. Администраторы публикуют релизы, управляют каналами, установщиками и доступом команды." />
      <div className="section-heading"><Title level={3}>Разделы платформы</Title><Text type="secondary">Каталог и распространение</Text></div>
      <div className="section-grid">
        {sections.slice(0, 4).map((section) => (
          <Link key={section.path} to={`/admin/${section.path}`} className="section-link">
            <Card className="section-card"><span className="section-icon">{section.icon}</span>
              <Title level={4}>{section.title}</Title><Paragraph>{section.description}</Paragraph>
              <span className="section-card-footer">{section.path === 'applications' ? 'Открыть каталог' : ['releases', 'artifacts'].includes(section.path) ? 'Выбрать приложение' : 'Запланировано'} <ArrowRightOutlined /></span>
            </Card>
          </Link>
        ))}
      </div>
    </>
  );
}
