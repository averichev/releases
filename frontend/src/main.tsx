import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { ConfigProvider } from 'antd';
import ruRU from 'antd/locale/ru_RU';
import { BrowserRouter } from 'react-router-dom';
import { AppRoutes } from './App';
import 'antd/dist/reset.css';
import './styles.css';

const root = document.getElementById('root');
if (!root) throw new Error('Root element is missing');

createRoot(root).render(
  <StrictMode>
    <ConfigProvider locale={ruRU} theme={{ token: {
      colorPrimary: '#236b54', colorInfo: '#236b54', borderRadius: 10,
      colorText: '#1e2c29', colorTextSecondary: '#687570',
      fontFamily: 'Inter, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif',
    } }}>
      <BrowserRouter><AppRoutes /></BrowserRouter>
    </ConfigProvider>
  </StrictMode>,
);
