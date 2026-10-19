import { useEffect, useRef, useState } from 'react';
import { ListChecks, Save } from 'lucide-react';
import { Badge, Spinner } from '../../components/ui';
import PageHeader from '../../components/PageHeader';
import { api } from '../../lib/tauri';
import { useAppStore } from '../../store';

/**
 * BuddyAI · 环境配置页（整体左右布局对齐国内 Buddy「环境配置」）。
 *
 * Docker 适配（移植自 main）：桌面客户端路径配置（workbuddy_ai / codebuddy_ai 自动
 * 检测与人工指定，api.env.locate）为桌面专属，整块裁剪——Docker 部署无本机客户端。
 * 保留任务配置（JWT Token 定时续期 / 每日自动打卡 / 积分数据同步，开关 + 每日时刻
 * 均可在本页修改）。应用入口可见性统一收敛到「系统设置 → 应用显示」，本页不再重复；
 * 模型目录开关归「资源调度」页。
 */

/** 任务行表单（开关 + 每日时刻） */
interface TaskForm {
  enabled: boolean;
  hhmm: string;
}

/** 任务配置行：开关 + 每日时刻（随「保存配置」统一生效） */
function TaskRow({
  title,
  desc,
  form,
  onChange,
}: {
  title: string;
  desc: string;
  form: TaskForm;
  onChange: (f: TaskForm) => void;
}) {
  return (
    <div className="rounded-lg border border-slate-100 p-3 dark:border-zinc-800">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <label className="flex cursor-pointer items-center gap-2 text-sm">
          <input
            type="checkbox"
            checked={form.enabled}
            onChange={(e) => onChange({ ...form, enabled: e.target.checked })}
          />
          <span className="font-medium">{title}</span>
          <Badge tone={form.enabled ? 'green' : 'amber'}>{form.enabled ? '已开启' : '未开启'}</Badge>
        </label>
        <div className="flex items-center gap-2">
          <span className="text-xs font-medium text-slate-500">每日时刻</span>
          <input
            type="time"
            value={form.hhmm}
            onChange={(e) => onChange({ ...form, hhmm: e.target.value || '00:00' })}
            disabled={!form.enabled}
            className="input h-8 !w-28 text-sm"
          />
        </div>
      </div>
      <p className="mt-1.5 text-[11px] leading-4 text-slate-400">{desc}</p>
    </div>
  );
}

export default function BuddyAiSettings() {
  const pushToast = useAppStore((s) => s.pushToast);
  const settings = useAppStore((s) => s.settings);
  const refreshSettings = useAppStore((s) => s.refreshSettings);

  // 任务表单（开关 + 时刻；存 app Settings，随「保存配置」统一提交）
  const [renewForm, setRenewForm] = useState<TaskForm>({ enabled: true, hhmm: '10:35' });
  const [checkinForm, setCheckinForm] = useState<TaskForm>({ enabled: false, hhmm: '09:20' });
  // 积分数据同步表单（五模式：每小时/每N小时/每N天/每日定时/关闭；存 app Settings）
  const [creditsForm, setCreditsForm] = useState({ mode: 'daily', hhmm: '23:45', hours: '2', days: '1' });
  const [saving, setSaving] = useState(false);
  // 设置快照只预填一次（保存后 refreshSettings 推新快照，不回灌正在编辑的表单）
  const prefilled = useRef(false);

  useEffect(() => {
    if (prefilled.current || !settings) return;
    prefilled.current = true;
    setRenewForm({
      enabled: settings.buddy_ai_renew_enabled ?? true,
      hhmm: settings.buddy_ai_renew_hhmm || '10:35',
    });
    setCheckinForm({
      enabled: settings.buddy_ai_checkin_enabled === true,
      hhmm: settings.buddy_ai_checkin_hhmm || '09:20',
    });
    setCreditsForm({
      mode: settings.buddy_ai_credits_sync_mode || (settings.buddy_ai_credits_sync_enabled === false ? 'off' : 'daily'),
      hhmm: settings.buddy_ai_credits_sync_hhmm || '23:45',
      hours: settings.buddy_ai_credits_sync_hours || '2',
      days: settings.buddy_ai_credits_sync_days || '1',
    });
  }, [settings]);

  const save = async () => {
    setSaving(true);
    // 间隔边界夹取 + 回写表单（审查 P3：input min/max 只是提示，"0"/"99" 可提交；
    // 后端虽有 clamp 兜底，但表单显示值会与落库值不一致）
    const hours = String(Math.min(Math.max(Number(creditsForm.hours) || 2, 1), 24));
    const days = String(Math.min(Math.max(Number(creditsForm.days) || 1, 1), 30));
    const nextForm = { ...creditsForm, hours, days };
    setCreditsForm(nextForm);
    try {
      await api.misc.settingsSet({
        buddy_ai_renew_enabled: renewForm.enabled,
        buddy_ai_renew_hhmm: renewForm.hhmm || '10:35',
        buddy_ai_checkin_enabled: checkinForm.enabled,
        buddy_ai_checkin_hhmm: checkinForm.hhmm || '09:20',
        buddy_ai_credits_sync_mode: nextForm.mode,
        buddy_ai_credits_sync_hhmm: nextForm.hhmm || '23:45',
        buddy_ai_credits_sync_hours: hours,
        buddy_ai_credits_sync_days: days,
        buddy_ai_credits_sync_enabled: nextForm.mode !== 'off',
      });
      await refreshSettings();
      pushToast('success', 'BuddyAI 环境配置已保存');
    } catch (err) {
      pushToast('error', `保存失败：${String(err).slice(0, 120)}`);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="animate-fade-in">
      <PageHeader
        title="BuddyAI · 环境配置"
        desc="BuddyAI 定时任务配置（打卡 / 续期 / 积分同步 / 目录同步）"
        actions={
          <button className="btn-outline" onClick={() => void save()} disabled={saving || !settings}>
            {saving ? <Spinner /> : <Save size={15} />} 保存配置
          </button>
        }
      />

      {/* 任务配置（Docker 适配：通用配置区为客户端路径本机态，已整块裁剪） */}
      <div className="grid items-start gap-4">
        <div className="card p-4">
          <div className="mb-3 flex items-center gap-2">
            <ListChecks size={16} className="text-emerald-500" />
            <h2 className="font-medium">任务配置</h2>
          </div>
          <div className="space-y-3">
            <TaskRow
              title="JWT Token 定时续期"
              desc="每日定时巡检，token 到期前 48h 内自动走 workbuddy.ai 刷新端点轮换（lazy 48h，未临期空转不计失败）；网关 401 亦会触发即时续期。"
              form={renewForm}
              onChange={setRenewForm}
            />
            <TaskRow
              title="每日自动打卡"
              desc="网页会话 + 沙箱跑一轮对话，服务端自动入账 30/50 积分（按套餐定档）；幂等跳过当日已完成账号；间隔等细项在「活跃打卡」页配置。"
              form={checkinForm}
              onChange={setCheckinForm}
            />
            {/* 积分数据同步（五模式对齐国内：每小时/每N小时/每N天/每日定时/关闭，随「保存配置」生效） */}
            <div className="rounded-lg border border-slate-100 p-3 dark:border-zinc-800">
              <div className="flex flex-wrap items-center gap-2 text-sm">
                <span className="font-medium">积分数据同步</span>
                <Badge tone={creditsForm.mode !== 'off' ? 'green' : 'amber'}>
                  {creditsForm.mode !== 'off' ? '已开启' : '已关闭'}
                </Badge>
                <span className="text-xs font-medium text-slate-500">同步模式</span>
                <select
                  className="input !h-9 !w-36 truncate !py-0 text-sm"
                  value={creditsForm.mode}
                  onChange={(e) => setCreditsForm({ ...creditsForm, mode: e.target.value })}
                >
                  <option value="every_hours">每N小时</option>
                  <option value="every_days">每N天</option>
                  <option value="hourly">每小时</option>
                  <option value="daily">每日定时</option>
                  <option value="off">关闭</option>
                </select>
                {creditsForm.mode === 'every_hours' && (
                  <>
                    <span className="text-xs font-medium text-slate-500">同步间隔</span>
                    <input
                      type="number"
                      min={1}
                      max={24}
                      className="input h-8 !w-16 text-sm"
                      value={creditsForm.hours}
                      onChange={(e) => setCreditsForm({ ...creditsForm, hours: e.target.value })}
                    />
                    <span className="text-xs text-slate-500">小时（1-24）</span>
                  </>
                )}
                {creditsForm.mode === 'every_days' && (
                  <>
                    <span className="text-xs font-medium text-slate-500">同步间隔</span>
                    <input
                      type="number"
                      min={1}
                      max={30}
                      className="input h-8 !w-16 text-sm"
                      value={creditsForm.days}
                      onChange={(e) => setCreditsForm({ ...creditsForm, days: e.target.value })}
                    />
                    <span className="text-xs text-slate-500">天（1-30）</span>
                  </>
                )}
                {creditsForm.mode === 'daily' && (
                  <>
                    <span className="text-xs font-medium text-slate-500">每日时刻</span>
                    <input
                      type="time"
                      value={creditsForm.hhmm}
                      onChange={(e) => setCreditsForm({ ...creditsForm, hhmm: e.target.value || '00:00' })}
                      className="input h-8 !w-28 text-sm"
                    />
                  </>
                )}
              </div>
              <p className="mt-1.5 text-[11px] leading-4 text-slate-400">
                应用运行期间按所选模式自动同步积分与 Token 看板数据（get-user-resource 全池取数 →
                积分快照 + Token 重扫 + 官网用量刷新）；间隔模式按「距上次成功执行满 N 小时/天」触发，
                无可用凭证时静默跳过。模式与间隔随右上角「保存配置」生效。
              </p>
            </div>
          </div>
          <div className="mt-3 rounded-md bg-slate-50 px-3 py-2 text-[11px] leading-4 text-slate-500 dark:bg-zinc-900 dark:text-zinc-400">
            任务开关与时刻改动随右上角「保存配置」统一生效；调度在应用运行期间执行（当天已过时刻、下次启动会自动补跑），应用关闭期间不执行。
            模型目录每日同步（默认 05:55）默认开启，开关在「资源调度」页。
          </div>
        </div>
      </div>
    </div>
  );
}
