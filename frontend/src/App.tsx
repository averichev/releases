import { lazy, Suspense } from 'react';
import {
  Button, Card, Layout, Menu, Result, Space, Spin, Tag, Typography,
} from 'antd';
import {
  DashboardOutlined, MenuOutlined, SafetyCertificateOutlined,
} from '@ant-design/icons';
import { Link, Navigate, Outlet, Route, Routes, useLocation } from 'react-router-dom';
import { sections } from './sections';

const { Title, Paragraph } = Typography;
const Overview = lazy(() => import('./pages/Overview'));

function AdminLayout() {
  const { pathname } = useLocation();
  const current = sections.find((section) => pathname.startsWith(`/admin/${section.path}`));
  return (
    <Layout className="app-layout">
      <Layout.Sider width={244} breakpoint="lg" collapsedWidth={0} className="sidebar" theme="light"
        trigger={<Button type="text" aria-label="Переключить навигацию" icon={<MenuOutlined />} />}>
        <Link to="/admin" className="brand" aria-label="Veruna Releases — обзор">
          <span className="brand-mark">v</span>
          <span>Veruna<span className="brand-subtitle">RELEASES</span></span>
        </Link>
        <div className="nav-label">РАСПРОСТРАНЕНИЕ ПО</div>
        <nav aria-label="Основная навигация">
          <Menu mode="inline" selectedKeys={[current?.path ?? 'overview']} items={[
            { key: 'overview', icon: <DashboardOutlined />, label: <Link to="/admin">Обзор</Link> },
            ...sections.map((section) => ({ key: section.path, icon: section.icon,
              label: <Link to={`/admin/${section.path}`}>{section.title}</Link> })),
          ]} />
        </nav>
        <div className="sidebar-note"><span className="status-dot" />Основа платформы<span>Этап 01</span></div>
      </Layout.Sider>
      <Layout className="main-layout">
        <Layout.Header className="topbar">
          <span>Панель управления <span className="breadcrumb-divider">/</span> <strong>{current?.title ?? 'Обзор'}</strong></span>
          <Tag variant="filled">Этап 1</Tag>
        </Layout.Header>
        <Layout.Content className="page-content"><Outlet /></Layout.Content>
        <Layout.Footer className="footer"><span>Veruna Releases</span><span>Платформа распространения ПО</span></Layout.Footer>
      </Layout>
    </Layout>
  );
}

function Placeholder({ title, description }: { title: string; description: string }) {
  return <>
    <div className="page-heading"><div className="eyebrow">ПАНЕЛЬ УПРАВЛЕНИЯ</div><Title level={1}>{title}</Title><Paragraph>{description}</Paragraph></div>
    <Card><Result icon={<SafetyCertificateOutlined />} title="Раздел запланирован"
      subTitle="Функциональность появится на следующих этапах. Данные пока не создаются и не обрабатываются."
      extra={<Link to="/admin"><Button>Вернуться к обзору</Button></Link>} /></Card>
  </>;
}

export function AppRoutes() {
  return <Routes>
    <Route path="/" element={<Navigate to="/admin" replace />} />
    <Route path="/admin" element={<AdminLayout />}>
      <Route index element={<Suspense fallback={<Spin aria-label="Загрузка обзора" />}><Overview /></Suspense>} />
      {sections.map((section) => <Route key={section.path} path={section.path} element={<Placeholder title={section.title} description={section.description} />} />)}
      <Route path="*" element={<Result status="404" title="Страница не найдена" extra={<Link to="/admin">Вернуться к обзору</Link>} />} />
    </Route>
    <Route path="*" element={<Space className="not-found"><Result status="404" title="Страница не найдена" extra={<Link to="/admin">Открыть панель</Link>} /></Space>} />
  </Routes>;
}
