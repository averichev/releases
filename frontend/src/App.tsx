import { lazy, Suspense } from 'react';
import {
  Button, Card, Layout, Menu, Result, Space, Spin, Tag, Typography,
} from 'antd';
import {
  DashboardOutlined, MenuOutlined, SafetyCertificateOutlined, UserOutlined,
} from '@ant-design/icons';
import { Link, Outlet, Route, Routes, useLocation } from 'react-router-dom';
import { AuthProvider, RequireSession, useAuth } from './auth/Auth';
import Login from './pages/Login';
import Activate from './pages/Activate';
import { sections } from './sections';
import { Brand } from './components/Brand';
import { ServiceVersion } from './components/ServiceVersion';

const { Title, Paragraph } = Typography;
const PublicApplication = lazy(() => import('./pages/PublicApplication'));
const PublicApplications = lazy(() => import('./pages/PublicApplications'));
const Users = lazy(() => import('./pages/Users'));
const Profile = lazy(() => import('./pages/Profile'));
const Overview = lazy(() => import('./pages/Overview'));
const ServiceUpdate = lazy(() => import('./pages/ServiceUpdate'));
const Applications = lazy(() => import('./pages/Applications'));
const ApplicationCreate = lazy(() => import('./pages/ApplicationCreate'));
const ReleaseDetail = lazy(() => import('./pages/ReleaseDetail'));
const ApplicationDetail = lazy(() => import('./pages/ApplicationDetail'));

function AdminLayout() {
  const { pathname } = useLocation();
  const { state } = useAuth();
  const visibleSections = sections.filter((s) => !['users', 'self-update'].includes(s.path) || (state.kind === 'ready' && state.user.role === 'admin'));
  const current = sections.find((section) => pathname.startsWith(`/admin/${section.path}`));
  return (
    <Layout className="app-layout">
      <Layout.Sider width={244} breakpoint="lg" collapsedWidth={0} className="sidebar" theme="light"
        trigger={<Button type="text" aria-label="Переключить навигацию" icon={<MenuOutlined />} />}>
        <Link to="/admin" className="brand" aria-label="Релизы — обзор">
          <Brand />
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
      </Layout.Sider>
      <Layout className="main-layout">
        <Layout.Header className="topbar">
          <span className="breadcrumb"><span className="breadcrumb-parent">Панель управления</span><span className="breadcrumb-divider">/</span> <strong>{pathname === '/admin/profile' ? 'Профиль' : current?.title ?? 'Обзор'}</strong></span>
          <Link to="/admin/profile"><Tag variant="filled"><UserOutlined /> {state.kind === 'ready' ? state.user.username : ''}</Tag></Link>
        </Layout.Header>
        <Layout.Content className="page-content"><Outlet /></Layout.Content>
        <Layout.Footer className="footer"><Brand /><span className="footer-description">Платформа распространения ПО</span><ServiceVersion /></Layout.Footer>
      </Layout>
    </Layout>
  );
}

function Placeholder({ title, description }: { title: string; description: string }) {
  const publicationTokens = title === 'API-токены';
  const withinApplication = title === 'Релизы' || title === 'Дистрибутивы' || publicationTokens;
  return <>
    <div className="page-heading"><div className="eyebrow">ПАНЕЛЬ УПРАВЛЕНИЯ</div><Title level={1}>{title}</Title><Paragraph>{description}</Paragraph></div>
    <Card><Result icon={<SafetyCertificateOutlined />} title={withinApplication ? "Выберите приложение" : "Раздел запланирован"}
      subTitle={publicationTokens ? "Администратор управляет токенами в Приложение → Настройки → Публикация из CLI. Каждый токен публикует новые выпуски только своего приложения." : withinApplication ? "Откройте приложение → Релизы для управления версиями и файлами." : "Функциональность появится на следующих этапах. Данные пока не создаются и не обрабатываются."}
      extra={<Link to={withinApplication ? "/admin/applications" : "/admin"}><Button>{withinApplication ? "Открыть приложения" : "Вернуться к обзору"}</Button></Link>} /></Card>
  </>;
}

export function AppRoutes() {
  return <Routes>
    <Route path="/" element={<Suspense fallback={<Spin />}><PublicApplications /></Suspense>} />
    <Route element={<AuthProvider><Outlet /></AuthProvider>}>
    <Route path="/admin/login" element={<Login />} />
    <Route path="/admin/activate" element={<Activate />} />
    <Route element={<RequireSession />}>
    <Route path="/admin" element={<AdminLayout />}>
      <Route index element={<Suspense fallback={<Spin aria-label="Загрузка обзора" />}><Overview /></Suspense>} />
      <Route path="users" element={<Suspense fallback={<Spin />}><Users /></Suspense>} />
      <Route path="self-update" element={<Suspense fallback={<Spin />}><ServiceUpdate /></Suspense>} />
      <Route path="profile" element={<Suspense fallback={<Spin />}><Profile /></Suspense>} />
      <Route path="applications" element={<Suspense fallback={<Spin />}><Applications /></Suspense>} />
      <Route path="applications/new" element={<Suspense fallback={<Spin />}><ApplicationCreate /></Suspense>} />
      <Route path="applications/:id/releases/:releaseId" element={<Suspense fallback={<Spin />}><ReleaseDetail /></Suspense>} />
      {['applications/:id', 'applications/:id/settings', 'applications/:id/releases', 'applications/:id/manifests', 'applications/:id/history', 'applications/:id/distribution', 'applications/:id/integrations/tauri'].map((path) => <Route key={path} path={path} element={<Suspense fallback={<Spin />}><ApplicationDetail /></Suspense>} />)}
      {sections.filter((s) => !['users', 'applications', 'self-update'].includes(s.path)).map((section) => <Route key={section.path} path={section.path} element={<Placeholder title={section.title} description={section.description} />} />)}
      <Route path="*" element={<Result status="404" title="Страница не найдена" extra={<Link to="/admin">Вернуться к обзору</Link>} />} />
    </Route>
    </Route>
    </Route>
    <Route path="/:slug" element={<Suspense fallback={<Spin />}><PublicApplication /></Suspense>} />
    <Route path="*" element={<Space className="not-found"><Result status="404" title="Страница не найдена" extra={<Link to="/admin">Открыть панель</Link>} /></Space>} />
  </Routes>;
}
