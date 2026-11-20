import type { ReactNode } from 'react';
import { Layout } from 'antd';
import { Link } from 'react-router-dom';
import { Brand } from './Brand';
import { ServiceVersion } from './ServiceVersion';

export function PublicLayout({ children }: { children: ReactNode }) {
  return <Layout className="public-layout">
    <Layout.Header className="public-header"><Link to="/" aria-label="Релизы — приложения"><Brand /></Link></Layout.Header>
    <Layout.Content className="public-content">{children}</Layout.Content>
    <Layout.Footer className="public-footer"><Brand /><span>Распространение приложений</span><ServiceVersion /></Layout.Footer>
  </Layout>;
}
