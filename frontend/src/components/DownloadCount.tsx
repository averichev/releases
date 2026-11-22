import { Tooltip } from 'antd';

export default function DownloadCount({ count }: { count: number }) {
  return <Tooltip title="Начала публичных скачиваний. HEAD и продолжение с другого байта не учитываются. Полная загрузка и уникальность пользователя не подтверждаются."><span>{count.toLocaleString('ru-RU')}</span></Tooltip>;
}
