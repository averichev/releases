import { useEffect, useState } from 'react';
import { Avatar, Button, Card, Empty, List, Pagination, Result, Spin, Typography } from 'antd';
import { AppstoreOutlined } from '@ant-design/icons';
import { Link } from 'react-router-dom';
import { api, errorMessage } from '../api/client';
import { PublicLayout } from '../components/PublicLayout';

interface PublicApplication { name: string; slug: string; icon_url: string | null }
interface Catalog { items: PublicApplication[]; total: number }
const pageSize = 20;

export default function PublicApplications() {
  const [catalog, setCatalog] = useState<Catalog>();
  const [page, setPage] = useState(1);
  const [error, setError] = useState('');
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    const c = new AbortController(); setCatalog(undefined); setError('');
    api<Catalog>(`/public/applications?limit=${pageSize}&offset=${(page - 1) * pageSize}`, 'GET', undefined, c.signal)
      .then((value) => { if (!c.signal.aborted) setCatalog(value); },
        (e) => { if (!c.signal.aborted) setError(errorMessage(e)); });
    return () => c.abort();
  }, [page, attempt]);
  return <PublicLayout>
    <div><Typography.Title level={1}>Приложения</Typography.Title>
      <Typography.Paragraph type="secondary">Выберите приложение, чтобы скачать актуальную версию и посмотреть историю релизов.</Typography.Paragraph></div>
    {error ? <Result status="error" title="Не удалось загрузить приложения" subTitle={error}
      extra={<Button onClick={() => setAttempt((value) => value + 1)}>Повторить</Button>} />
      : !catalog ? <div className="catalog-loading"><Spin aria-label="Загрузка приложений" /></div>
        : !catalog.total ? <Empty description="Опубликованных приложений пока нет" />
          : <><Card className="public-catalog"><List dataSource={catalog.items} renderItem={(app) => <List.Item key={app.slug}>
            <Link to={`/${app.slug}`} className="public-application-link">
              <Avatar className="application-icon" shape="square" size={64} src={app.icon_url} icon={<AppstoreOutlined />} />
              <Typography.Title level={2}>{app.name}</Typography.Title>
            </Link>
          </List.Item>} /></Card>
          {catalog.total > pageSize && <Pagination current={page} pageSize={pageSize} total={catalog.total} onChange={setPage} showSizeChanger={false} />}</>}
  </PublicLayout>;
}
