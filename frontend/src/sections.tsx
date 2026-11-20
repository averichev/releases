import {
  AppstoreOutlined, CloudDownloadOutlined,
  KeyOutlined, RocketOutlined, TeamOutlined, LineChartOutlined,
} from '@ant-design/icons';

// Navigation and route placeholders share one definition to prevent drift.
export const sections = [
  { path: 'applications', title: 'Приложения', icon: <AppstoreOutlined />,
    description: 'Каталог распространяемых продуктов и их настройки.' },
  { path: 'releases', title: 'Релизы', icon: <RocketOutlined />,
    description: 'Черновики и опубликованные версии в карточках приложений.' },
  { path: 'artifacts', title: 'Дистрибутивы', icon: <CloudDownloadOutlined />,
    description: 'Установщики и контрольные суммы внутри релизов.' },

  { path: 'users', title: 'Пользователи', icon: <TeamOutlined />,
    description: 'Участники команды и права доступа к управлению сервисом.' },
  { path: 'api-tokens', title: 'API-токены', icon: <KeyOutlined />,
    description: 'Доступ автоматизированных систем к административному API.' },
  { path: 'downloads', title: 'Скачивания', icon: <LineChartOutlined />,
    description: 'События скачивания и статистика распространения.' },
];
