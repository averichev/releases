import {
  AppstoreOutlined, CloudDownloadOutlined,
  KeyOutlined, RocketOutlined, TeamOutlined, LineChartOutlined, SyncOutlined,
} from '@ant-design/icons';

// Navigation and route placeholders share one definition to prevent drift.
export const sections = [
  { path: 'applications', title: 'Приложения', icon: <AppstoreOutlined />,
    description: 'Каталог распространяемых продуктов и их настройки.' },
  { path: 'releases', title: 'Релизы', icon: <RocketOutlined />,
    description: 'Черновики и опубликованные версии в карточках приложений.' },
  { path: 'artifacts', title: 'Дистрибутивы', icon: <CloudDownloadOutlined />,
    description: 'Файлы релизов и их контрольные суммы.' },

  { path: 'users', title: 'Пользователи', icon: <TeamOutlined />,
    description: 'Участники команды и права доступа к управлению сервисом.' },
  { path: 'api-tokens', title: 'API-токены', icon: <KeyOutlined />,
    description: 'Публикация из curl и CI: токены выдаются в настройках приложения.' },
  { path: 'downloads', title: 'Скачивания', icon: <LineChartOutlined />,
    description: 'События скачивания и статистика распространения.' },
  { path: 'self-update', title: 'Обновление сервиса', icon: <SyncOutlined />,
    description: 'Подписанные версии Veruna Releases.' },
];
