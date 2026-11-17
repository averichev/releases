import { useMemo } from 'react';
import { App, Button } from 'antd';
import hljs from 'highlight.js/lib/core';
import json from 'highlight.js/lib/languages/json';
import typescript from 'highlight.js/lib/languages/typescript';
import rust from 'highlight.js/lib/languages/rust';
import bash from 'highlight.js/lib/languages/bash';
hljs.registerLanguage('json', json); hljs.registerLanguage('typescript', typescript);
hljs.registerLanguage('rust', rust); hljs.registerLanguage('bash', bash);
export default function CodeBlock({ code, language = 'bash', title }: {code: string; language?: 'json' | 'typescript' | 'rust' | 'bash'; title: string}) {
  const { message } = App.useApp();
  const html = useMemo(() => hljs.highlight(code, {language, ignoreIllegals: true}).value, [code, language]);
  async function copy() {
    try { await navigator.clipboard.writeText(code); void message.success('Скопировано'); }
    catch { void message.error('Не удалось скопировать. Выделите текст вручную.'); }
  }
  return <div className="integration-code"><div className="integration-code-heading"><span>{title}</span><Button size="small" onClick={() => {void copy();}} aria-label={`Копировать ${title}`}>Копировать</Button></div>
    <pre aria-label={title}><code className="hljs" dangerouslySetInnerHTML={{__html: html}} /></pre></div>;
}
