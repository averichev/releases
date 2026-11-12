import { lazy, Suspense } from 'react';
import {
  Button, Card, Layout, Menu, Result, Space, Spin, Tag, Typography,
} from 'antd';
import {
  DashboardOutlined, MenuOutlined, SafetyCertificateOutlined, UserOutlined,
} from '@ant-design/icons';
import { Link, Navigate, Outlet, Route, Routes, useLocation } from 'react-router-dom';
import { AuthProvider, RequireSession, useAuth } from './auth/Auth';
import Login from './pages/Login';
import Activate from './pages/Activate';
import { sections } from './sections';

const { Title, Paragraph } = Typography;
const Users = lazy(() => import('./pages/Users'));
const Profile = lazy(() => import('./pages/Profile'));
const Overview = lazy(() => import('./pages/Overview'));
const Applications = lazy(() => import('./pages/Applications'));
const ApplicationCreate = lazy(() => import('./pages/ApplicationCreate'));
const ReleaseDetail = lazy(() => import('./pages/ReleaseDetail'));
const ApplicationDetail = lazy(() => import('./pages/ApplicationDetail'));

function AdminLayout() {
  const { pathname } = useLocation();
  const { state } = useAuth();
  const visibleSections = sections.filter((s) => s.path !== 'users' || (state.kind === 'ready' && state.user.role === 'admin'));
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
          <Menu mode="inline" selectedKeys={[pathname === '/admin/profile' ? 'profile' : current?.path ?? 'overview']} items={[
            { key: 'overview', icon: <DashboardOutlined />, label: <Link to="/admin">Обзор</Link> },
            ...visibleSections.map((section) => ({ key: section.path, icon: section.icon,
              label: <Link to={`/admin/${section.path}`}>{section.title}</Link> })),
            { key: 'profile', icon: <UserOutlined />, label: <Link to="/admin/profile">Профиль</Link> },
          ]} />
        </nav>
        <div className="sidebar-note"><span className="status-dot" />Защищённый доступ<span>Этап 04</span></div>
      </Layout.Sider>
      <Layout className="main-layout">
        <Layout.Header className="topbar">
          <span className="breadcrumb"><span className="breadcrumb-parent">Панель управления</span><span className="breadcrumb-divider">/</span> <strong>{pathname === '/admin/profile' ? 'Профиль' : current?.title ?? 'Обзор'}</strong></span>
          <Link to="/admin/profile"><Tag variant="filled"><UserOutlined /> {state.kind === 'ready' ? state.user.username : ''}</Tag></Link>
        </Layout.Header>
        <Layout.Content className="page-content"><Outlet /></Layout.Content>
        <Layout.Footer className="footer"><span>Veruna Releases</span><span>Платформа распространения ПО</span></Layout.Footer>
      </Layout>
    </Layout>
  );
}

function Placeholder({ title, description }: { title: string; description: string }) {
  const withinApplication = title === 'Релизы' || title === 'Дистрибутивы';
  return <>
    <div className="page-heading"><div className="eyebrow">ПАНЕЛЬ УПРАВЛЕНИЯ</div><Title level={1}>{title}</Title><Paragraph>{description}</Paragraph></div>
    <Card><Result icon={<SafetyCertificateOutlined />} title={withinApplication ? "Выберите приложение" : "Раздел запланирован"}
      subTitle={withinApplication ? "Откройте приложение → Релизы. В каждом черновике можно управлять установщиками." : "Функциональность появится на следующих этапах. Данные пока не создаются и не обрабатываются."}
      extra={<Link to={withinApplication ? "/admin/applications" : "/admin"}><Button>{withinApplication ? "Открыть приложения" : "Вернуться к обзору"}</Button></Link>} /></Card>
  </>;
}

export function AppRoutes() {
  return <AuthProvider><Routes>
    <Route path="/" element={<Navigate to="/admin" replace />} />
    <Route path="/admin/login" element={<Login />} />
    <Route path="/admin/activate" element={<Activate />} />
    <Route element={<RequireSession />}>
    <Route path="/admin" element={<AdminLayout />}>
      <Route index element={<Suspense fallback={<Spin aria-label="Загрузка обзора" />}><Overview /></Suspense>} />
      <Route path="users" element={<Suspense fallback={<Spin />}><Users /></Suspense>} />
      <Route path="profile" element={<Suspense fallback={<Spin />}><Profile /></Suspense>} />
      <Route path="applications" element={<Suspense fallback={<Spin />}><Applications /></Suspense>} />
      <Route path="applications/new" element={<Suspense fallback={<Spin />}><ApplicationCreate /></Suspense>} />
      <Route path="applications/:id/releases/:releaseId" element={<Suspense fallback={<Spin />}><ReleaseDetail /></Suspense>} />
      {['applications/:id', 'applications/:id/settings', 'applications/:id/releases'].map((path) => <Route key={path} path={path} element={<Suspense fallback={<Spin />}><ApplicationDetail /></Suspense>} />)}
      {sections.filter((s) => s.path !== 'users' && s.path !== 'applications').map((section) => <Route key={section.path} path={section.path} element={<Placeholder title={section.title} description={section.description} />} />)}
      <Route path="*" element={<Result status="404" title="Страница не найдена" extra={<Link to="/admin">Вернуться к обзору</Link>} />} />
    </Route>
    </Route>
    <Route path="*" element={<Space className="not-found"><Result status="404" title="Страница не найдена" extra={<Link to="/admin">Открыть панель</Link>} /></Space>} />
  </Routes></AuthProvider>;
}
