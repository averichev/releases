import { Typography } from 'antd';
import Markdown from 'react-markdown';

/** Release notes are untrusted content: no raw HTML or remote image requests. */
export default function ReleaseNotes({ children }: { children: string }) {
  return <Typography className="release-notes"><Markdown skipHtml disallowedElements={['img']}>{children}</Markdown></Typography>;
}
