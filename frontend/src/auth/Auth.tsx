import { createContext, useCallback, useContext, useEffect, useState, useRef, type ReactNode } from 'react';
import { Alert, Button, Spin } from 'antd';
import { Navigate, Outlet, useLocation } from 'react-router-dom';
import { api, ApiError, errorMessage, setCsrf, type Session, type User } from '../api/client';
type State = { kind: 'loading' } | { kind: 'anonymous' } | { kind: 'ready'; user: User } | { kind: 'error'; message: string };
interface Auth { state: State; refresh: () => Promise<void>; login: (username: string, password: string) => Promise<void>; logout: () => Promise<void> }
const Context = createContext<Auth | null>(null);
export function AuthProvider({ children }: { children: ReactNode }) {
  const sequence = useRef(0);
  const [state, setState] = useState<State>({ kind: 'loading' });
  const refresh = useCallback(async () => {
    const current = ++sequence.current;
    try { const session = await api<Session>('/auth/me'); if (sequence.current !== current) return; setCsrf(session.csrf_token); setState({ kind: 'ready', user: session.user }); }
    catch (error) {
      if (sequence.current !== current) return;
      setCsrf();
      setState(error instanceof ApiError && error.status === 401 ? { kind: 'anonymous' } : { kind: 'error', message: errorMessage(error) });
    }
  }, []);
  useEffect(() => {
    void refresh();
    const expired = () => { ++sequence.current; setCsrf(); setState({ kind: 'anonymous' }); };
    const focus = () => { void refresh(); };
    window.addEventListener('session-expired', expired); window.addEventListener('focus', focus);
    return () => { window.removeEventListener('session-expired', expired); window.removeEventListener('focus', focus); };
  }, [refresh]);
  async function login(username: string, password: string) {
    ++sequence.current;
    const session = await api<Session>('/auth/login', 'POST', { username, password });
    ++sequence.current; setCsrf(session.csrf_token); setState({ kind: 'ready', user: session.user });
  }
  async function logout() { ++sequence.current; await api('/auth/logout', 'POST', {}); ++sequence.current; setCsrf(); setState({ kind: 'anonymous' }); }
  return <Context.Provider value={{ state, refresh, login, logout }}>{children}</Context.Provider>;
}
export function useAuth() { const value = useContext(Context); if (!value) throw new Error('AuthProvider required'); return value; }
export function SessionLoading() { return <div className="session-loading" role="status"><Spin size="large" /><span>Проверяем сессию…</span></div>; }
export function RequireSession() {
  const { state, refresh } = useAuth(); const location = useLocation();
  if (state.kind === 'loading') return <SessionLoading />;
  if (state.kind === 'error') return <div className="auth-page"><Alert type="error" showIcon title="Не удалось проверить сессию" description={state.message}
    action={<Button onClick={() => { void refresh(); }}>Повторить</Button>} /></div>;
  if (state.kind === 'anonymous') return <Navigate to="/admin/login" state={{ from: location.pathname + location.search }} replace />;
  return <Outlet />;
}
