import { Snowflake } from 'lucide-react';
import { Badge } from '../../components/ui';

const COOLDOWN_LABELS: Record<string, string> = {
  PlanLimit: '套餐限额',
  SoftRate: '限流',
  SessionDead: '登录已失效',
  NotFound: '接口异常',
  Server: '服务端错误',
  Client: '客户端错误',
  BusinessError: '业务错误',
  DeviceSigned: '设备已签',
  AuthRejected: '登录已被拒绝',
};

/** SessionDead（JWT 被服务端吊销）的恢复指引：重新 OAuth 登录该账号即可恢复 */
const SESSION_DEAD_GUIDE =
  '该账号的登录已失效（JWT 被服务端吊销，常见于账号在别处重新登录或触发风控）。' +
  '恢复方式：回到「账号管理」点「OAuth 登录」重新授权添加该账号（同账号会更新凭证），签到即可恢复。';

/** AuthRejected（服务端拒绝认证，非网络/限流）的恢复指引：JWT 存在但被服务端判定无效，需重新登录 */
const AUTH_REJECTED_GUIDE =
  '服务端拒绝了该账号的登录凭证（11128：请求被拒绝，非网络或限流问题）。JWT 存在但被判定无效，' +
  '常见于服务端已标记为不可用（设备/环境风控名单）。恢复方式：回到「账号管理」点「OAuth 登录」' +
  '重新授权添加该账号。';

export function CooldownBadge({ type, until }: { type: string; until: number | null }) {
  const label = COOLDOWN_LABELS[type] ?? type;
  const isPermanent = type === 'SessionDead' || type === 'AuthRejected';
  let remaining = '';
  if (!isPermanent && until) {
    const secs = until - Math.floor(Date.now() / 1000);
    if (secs > 0) {
      const h = Math.floor(secs / 3600);
      const m = Math.floor((secs % 3600) / 60);
      remaining = h > 0 ? `${h}h${m}m` : `${m}m`;
    }
  }
  const title = type === 'SessionDead'
    ? SESSION_DEAD_GUIDE
    : type === 'AuthRejected'
      ? AUTH_REJECTED_GUIDE
      : undefined;
  return (
    <Badge tone={isPermanent ? 'red' : 'amber'} title={title}>
      <Snowflake size={12} /> {label}{remaining && ` ${remaining}`}
    </Badge>
  );
}
