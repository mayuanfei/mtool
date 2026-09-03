import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  AlertTriangle, BookOpen, CheckCircle2, ChevronRight,
  ClipboardCheck, Clock3, ExternalLink, FileText, Gauge, GraduationCap,
  Loader2, Pause, Play, Presentation, RefreshCw, RotateCcw, ShieldCheck, SkipForward, Trash2,
  Video, Volume2, VolumeX,
} from 'lucide-react';

type Provider = 'ulearn' | 'merchant';
type CourseKind = 'video' | 'exam' | 'slides' | 'material';
type CourseStatus = 'completed' | 'pending' | 'opening' | 'playing' | 'verifying' | 'manual' | 'attention';

interface VideoTaskSettings {
  speed: number;
  muted: boolean;
  crossSiteParallel: boolean;
  running: boolean;
}

interface SourceStatus {
  provider: Provider;
  name: string;
  homeUrl: string;
  windowOpen: boolean;
  currentUrl: string | null;
}

interface CourseItem {
  id: string;
  title: string;
  url: string;
  sectionTitle: string;
  kind: CourseKind;
  durationSeconds: number;
  progress: number;
  status: CourseStatus;
  lastError: string | null;
}

interface TopicItem {
  id: string;
  provider: Provider;
  title: string;
  url: string;
  progress: number;
  totalCount: number;
  completedCount: number;
  lastSyncedAt: number;
  courses: CourseItem[];
}

interface QueueStats {
  total: number;
  completed: number;
  pending: number;
  running: number;
  manual: number;
  attention: number;
}

interface VideoTaskDashboard {
  settings: VideoTaskSettings;
  sources: SourceStatus[];
  topics: TopicItem[];
  stats: QueueStats;
}

interface ImportSummary {
  topicId: string;
  topicTitle: string;
  imported: number;
  completed: number;
  manual: number;
}

const EMPTY_DASHBOARD: VideoTaskDashboard = {
  settings: { speed: 2, muted: true, crossSiteParallel: false, running: false },
  sources: [],
  topics: [],
  stats: { total: 0, completed: 0, pending: 0, running: 0, manual: 0, attention: 0 },
};

const STATUS_META: Record<CourseStatus, { label: string; classes: string }> = {
  completed: { label: '已完成', classes: 'text-emerald-400 bg-emerald-500/10 border-emerald-500/25' },
  pending: { label: '待播放', classes: 'text-sky-400 bg-sky-500/10 border-sky-500/25' },
  opening: { label: '正在打开', classes: 'text-sky-400 bg-sky-500/10 border-sky-500/25' },
  playing: { label: '正在播放', classes: 'text-indigo-400 bg-indigo-500/10 border-indigo-500/25' },
  verifying: { label: '完成核验中', classes: 'text-amber-400 bg-amber-500/10 border-amber-500/25' },
  manual: { label: '需本人处理', classes: 'text-orange-400 bg-orange-500/10 border-orange-500/25' },
  attention: { label: '需要处理', classes: 'text-rose-400 bg-rose-500/10 border-rose-500/25' },
};

const KIND_META: Record<CourseKind, { label: string; icon: typeof Video; classes: string; badgeClasses: string }> = {
  video: {
    label: '课程视频',
    icon: Video,
    classes: 'text-indigo-400 bg-indigo-500/10 border-indigo-500/25',
    badgeClasses: 'text-indigo-400 bg-indigo-500/10 border-indigo-500/25',
  },
  slides: {
    label: 'PPT 课件',
    icon: Presentation,
    classes: 'text-amber-400 bg-amber-500/10 border-amber-500/25',
    badgeClasses: 'text-amber-400 bg-amber-500/10 border-amber-500/25',
  },
  material: {
    label: '文档资料',
    icon: FileText,
    classes: 'text-emerald-400 bg-emerald-500/10 border-emerald-500/25',
    badgeClasses: 'text-emerald-400 bg-emerald-500/10 border-emerald-500/25',
  },
  exam: {
    label: '课程考试',
    icon: ClipboardCheck,
    classes: 'text-rose-400 bg-rose-500/10 border-rose-500/25',
    badgeClasses: 'text-rose-400 bg-rose-500/10 border-rose-500/25',
  },
};

function resolveCourseKind(course: { kind?: string; title?: string }): CourseKind {
  const title = (course.title || '').trim();
  if (/考试|测验|测试/.test(title)) return 'exam';
  if (/ppt|课件|幻灯片|演示/i.test(title)) return 'slides';
  if (/文档|阅读材料|参考资料|资料|pdf|手册/i.test(title)) return 'material';
  if (course.kind === 'exam' || course.kind === 'slides' || course.kind === 'material') {
    return course.kind;
  }
  return 'video';
}

function cx(...values: Array<string | false | null | undefined>): string {
  return values.filter(Boolean).join(' ');
}

function formatDuration(seconds: number): string {
  if (seconds <= 0) return '时长未知';
  const minutes = Math.ceil(seconds / 60);
  if (minutes < 60) return String(minutes) + ' 分钟';
  return String(Math.floor(minutes / 60)) + ' 小时 ' + String(minutes % 60) + ' 分';
}

function formatSyncTime(timestamp: number): string {
  return timestamp ? new Date(timestamp * 1000).toLocaleString() : '尚未同步';
}

function currentHost(url: string | null): string {
  if (!url) return '';
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

export function VideoTasks() {
  const [dashboard, setDashboard] = useState<VideoTaskDashboard>(EMPTY_DASHBOARD);
  const [loading, setLoading] = useState(true);
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [expandedTopics, setExpandedTopics] = useState<Set<string>>(new Set());
  const [deleteConfirmTopic, setDeleteConfirmTopic] = useState<TopicItem | null>(null);
  const [message, setMessage] = useState<{ text: string; error: boolean } | null>(null);
  const tickRunning = useRef(false);
  const initialLoadedRef = useRef(false);

  const showMessage = useCallback((text: string, error = false) => {
    setMessage({ text, error });
    window.setTimeout(() => setMessage(null), 4500);
  }, []);

  const refreshDashboard = useCallback(async () => {
    try {
      const next = await invoke<VideoTaskDashboard>('get_video_task_dashboard');
      setDashboard(next);
      if (!initialLoadedRef.current) {
        initialLoadedRef.current = true;
        if (next.topics.length > 0) {
          setExpandedTopics(new Set([next.topics[0].id]));
        }
      }
    } catch (error) {
      showMessage(String(error), true);
    } finally {
      setLoading(false);
    }
  }, [showMessage]);

  const tickAndRefresh = useCallback(async () => {
    if (tickRunning.current) return;
    tickRunning.current = true;
    try {
      await invoke('tick_video_queue');
      await refreshDashboard();
    } catch (error) {
      showMessage(String(error), true);
    } finally {
      tickRunning.current = false;
    }
  }, [refreshDashboard, showMessage]);

  useEffect(() => {
    refreshDashboard();
    const timer = window.setInterval(tickAndRefresh, 2500);
    window.addEventListener('focus', refreshDashboard);
    return () => {
      window.clearInterval(timer);
      window.removeEventListener('focus', refreshDashboard);
    };
  }, [refreshDashboard, tickAndRefresh]);

  const overallProgress = useMemo(() => (
    dashboard.stats.total > 0
      ? Math.round((dashboard.stats.completed / dashboard.stats.total) * 100)
      : 0
  ), [dashboard.stats.completed, dashboard.stats.total]);

  const runAction = async (key: string, action: () => Promise<void>, success?: string) => {
    setBusyKey(key);
    try {
      await action();
      if (success) showMessage(success);
      await refreshDashboard();
    } catch (error) {
      showMessage(String(error), true);
    } finally {
      setBusyKey(null);
    }
  };

  const openSite = (provider: Provider) => runAction(
    'open-' + provider,
    () => invoke('open_video_learning_site', { provider }),
  );

  const importTopic = (provider: Provider) => runAction(
    'import-' + provider,
    async () => {
      const result = await invoke<ImportSummary>('import_current_video_topic', { provider });
      showMessage(
        '已导入“' + result.topicTitle + '”：' + String(result.imported) +
        ' 项，已完成 ' + String(result.completed) + ' 项，需本人处理 ' + String(result.manual) + ' 项。',
      );
    },
  );

  const syncTopic = (topic: TopicItem) => runAction(
    'sync-' + topic.id,
    async () => {
      const result = await invoke<ImportSummary>('sync_video_topic', { topicId: topic.id });
      showMessage('已同步“' + result.topicTitle + '”的最新平台状态。');
    },
  );

  const updateSettings = (patch: Partial<VideoTaskSettings>) => {
    const next = { ...dashboard.settings, ...patch };
    setDashboard((current) => ({ ...current, settings: next }));
    void runAction('settings', () => invoke('update_video_task_settings', { settings: next }));
  };

  const toggleTopic = (topicId: string) => {
    setExpandedTopics((current) => {
      const next = new Set(current);
      if (next.has(topicId)) next.delete(topicId);
      else next.add(topicId);
      return next;
    });
  };

  const activeTasks = useMemo(() => {
    const list: Array<{ topicId: string; topicTitle: string; course: CourseItem }> = [];
    for (const topic of dashboard.topics) {
      for (const course of topic.courses) {
        if (course.status === 'opening' || course.status === 'playing' || course.status === 'verifying') {
          list.push({ topicId: topic.id, topicTitle: topic.title, course });
        }
      }
    }
    return list;
  }, [dashboard.topics]);

  if (loading) {
    return (
      <div className="h-full flex items-center justify-center th-text-muted">
        <Loader2 className="w-5 h-5 animate-spin mr-2" />
        正在加载学习任务…
      </div>
    );
  }

  return (
    <div className="max-w-7xl mx-auto w-full pb-10">
      <header className="mb-6 flex flex-col xl:flex-row xl:items-end xl:justify-between gap-5">
        <div className="flex items-center gap-3">
          <div className="w-11 h-11 rounded-xl bg-indigo-500/15 border border-indigo-500/25 flex items-center justify-center">
            <GraduationCap className="w-6 h-6 text-indigo-400" />
          </div>
          <div>
            <h1 className="text-3xl font-bold tracking-tight th-text">学习任务管理台</h1>
            <p className="text-sm th-text-3 mt-1">导入专题、顺序播放课程视频，并以平台状态作为最终完成依据。</p>
          </div>
        </div>
        <div className="flex flex-wrap gap-2">
          <div className={cx(
            'px-3 py-2 rounded-lg border text-xs font-semibold flex items-center gap-2',
            dashboard.settings.running
              ? 'bg-emerald-500/10 border-emerald-500/30 text-emerald-400'
              : 'th-bg-card th-border th-text-muted',
          )}>
            <span className={cx(
              'w-2 h-2 rounded-full',
              dashboard.settings.running ? 'bg-emerald-400 animate-pulse' : 'bg-slate-500',
            )} />
            {dashboard.settings.running ? '队列运行中' : '队列已暂停'}
          </div>
          <button
            onClick={() => void runAction('refresh', refreshDashboard)}
            className="px-3 py-2 rounded-lg border th-border th-bg-card th-text-3 th-hover-surface text-xs font-semibold flex items-center gap-2"
          >
            <RefreshCw className={cx('w-3.5 h-3.5', busyKey === 'refresh' && 'animate-spin')} />
            刷新
          </button>
        </div>
      </header>

      {message && (
        <div className={cx(
          'mb-5 px-4 py-3 rounded-xl border text-sm',
          message.error
            ? 'bg-rose-500/10 border-rose-500/30 text-rose-300'
            : 'bg-emerald-500/10 border-emerald-500/30 text-emerald-300',
        )}>
          {message.text}
        </div>
      )}

      <section className="grid grid-cols-1 xl:grid-cols-2 gap-4 mb-5">
        {dashboard.sources.map((source) => (
          <div key={source.provider} className="th-bg-card border th-border rounded-xl p-5 shadow-xl">
            <div className="flex items-start justify-between gap-4">
              <div className="flex items-start gap-3 min-w-0">
                <div className={cx(
                  'w-10 h-10 rounded-lg flex items-center justify-center border',
                  source.windowOpen
                    ? 'bg-emerald-500/10 border-emerald-500/25 text-emerald-400'
                    : 'th-bg-surface th-border-subtle th-text-muted',
                )}>
                  <BookOpen className="w-5 h-5" />
                </div>
                <div className="min-w-0">
                  <h2 className="font-semibold th-text-2">{source.name}</h2>
                  <div className="text-xs th-text-muted mt-1 truncate">
                    {source.windowOpen ? '会话已打开 · ' + currentHost(source.currentUrl) : '尚未打开登录会话'}
                  </div>
                  <div className="text-[11px] th-text-faint mt-1 truncate select-text">{source.homeUrl}</div>
                </div>
              </div>
              <span className={cx(
                'text-[11px] px-2 py-1 rounded-md border',
                source.currentUrl?.toLowerCase().includes('/login') || source.currentUrl?.toLowerCase().includes('/sso')
                  ? 'text-amber-400 border-amber-500/25 bg-amber-500/10'
                  : dashboard.settings.running && source.windowOpen
                  ? 'text-indigo-400 border-indigo-500/25 bg-indigo-500/10'
                  : source.windowOpen
                  ? 'text-emerald-400 border-emerald-500/25 bg-emerald-500/10'
                  : 'th-text-muted th-border',
              )}>
                {source.currentUrl?.toLowerCase().includes('/login') || source.currentUrl?.toLowerCase().includes('/sso')
                  ? '需扫码/登录'
                  : dashboard.settings.running && source.windowOpen
                  ? '静默运行中'
                  : source.windowOpen
                  ? '会话已连接'
                  : '未连接'}
              </span>
            </div>
            <div className="mt-4 flex flex-wrap gap-2">
              <button
                onClick={() => void openSite(source.provider)}
                disabled={busyKey === 'open-' + source.provider}
                className={cx(
                  'px-3 py-2 disabled:opacity-50 text-white rounded-lg text-xs font-semibold flex items-center gap-2',
                  source.currentUrl?.toLowerCase().includes('/login') || source.currentUrl?.toLowerCase().includes('/sso')
                    ? 'bg-amber-600 hover:bg-amber-500'
                    : 'bg-indigo-600 hover:bg-indigo-500'
                )}
              >
                {busyKey === 'open-' + source.provider
                  ? <Loader2 className="w-3.5 h-3.5 animate-spin" />
                  : <ExternalLink className="w-3.5 h-3.5" />}
                {source.currentUrl?.toLowerCase().includes('/login') || source.currentUrl?.toLowerCase().includes('/sso')
                  ? '打开完成登录'
                  : source.windowOpen ? '打开/选择专题' : '登录并选择专题'}
              </button>
              <button
                onClick={() => void importTopic(source.provider)}
                disabled={!source.windowOpen || busyKey === 'import-' + source.provider}
                className="px-3 py-2 th-bg-input-alt border th-border-subtle th-text-2 rounded-lg text-xs font-semibold flex items-center gap-2 disabled:opacity-40 th-hover-surface"
              >
                {busyKey === 'import-' + source.provider
                  ? <Loader2 className="w-3.5 h-3.5 animate-spin" />
                  : <RefreshCw className="w-3.5 h-3.5" />}
                导入当前专题
              </button>
            </div>
          </div>
        ))}
      </section>

      <section className="grid grid-cols-2 md:grid-cols-3 xl:grid-cols-6 gap-3 mb-5">
        {[
          { label: '整体进度', value: String(overallProgress) + '%', color: 'text-indigo-400' },
          { label: '课程总数', value: dashboard.stats.total, color: 'th-text' },
          { label: '已完成', value: dashboard.stats.completed, color: 'text-emerald-400' },
          { label: '待播放', value: dashboard.stats.pending, color: 'text-sky-400' },
          { label: '需本人处理', value: dashboard.stats.manual, color: 'text-orange-400' },
          { label: '异常', value: dashboard.stats.attention, color: 'text-rose-400' },
        ].map((item) => (
          <div key={item.label} className="th-bg-card border th-border rounded-xl px-4 py-3">
            <div className="text-[11px] th-text-muted">{item.label}</div>
            <div className={cx('text-2xl font-bold tabular-nums mt-1', item.color)}>{item.value}</div>
          </div>
        ))}
      </section>

      <section className="th-bg-card border th-border rounded-xl p-5 mb-5 shadow-xl">
        <div className="flex flex-col xl:flex-row xl:items-center xl:justify-between gap-5">
          <div>
            <div className="flex items-center gap-2 mb-2">
              <Gauge className="w-4 h-4 text-indigo-400" />
              <h2 className="text-sm font-semibold th-text-2">队列运行策略</h2>
            </div>
            <p className="text-xs th-text-muted">同一平台始终只运行一门课程；跨站并行开启后，两个平台可各运行一门。</p>
          </div>
          <div className="flex flex-wrap items-center gap-4">
            <div className="flex items-center gap-1 p-1 rounded-lg th-bg-input border th-border-subtle">
              {[1, 1.5, 2].map((speed) => (
                <button
                  key={speed}
                  onClick={() => updateSettings({ speed })}
                  className={cx(
                    'px-3 py-1.5 rounded-md text-xs font-semibold transition-colors',
                    dashboard.settings.speed === speed ? 'bg-indigo-600 text-white' : 'th-text-3 th-hover-surface',
                  )}
                >
                  {speed}×
                </button>
              ))}
            </div>
            <button
              onClick={() => updateSettings({ muted: !dashboard.settings.muted })}
              className={cx(
                'px-3 py-2 rounded-lg border text-xs font-semibold flex items-center gap-2',
                dashboard.settings.muted
                  ? 'bg-rose-500/10 border-rose-500/25 text-rose-400'
                  : 'th-bg-input-alt th-border-subtle th-text-2',
              )}
            >
              {dashboard.settings.muted ? <VolumeX className="w-3.5 h-3.5" /> : <Volume2 className="w-3.5 h-3.5" />}
              {dashboard.settings.muted ? '静音' : '声音开启'}
            </button>
            <label className="flex items-center gap-2 text-xs th-text-3 cursor-pointer">
              <input
                type="checkbox"
                checked={dashboard.settings.crossSiteParallel}
                onChange={(event) => updateSettings({ crossSiteParallel: event.target.checked })}
                className="accent-indigo-500"
              />
              跨站并行（最多 2 路）
            </label>
            {dashboard.settings.running && (dashboard.stats.running > 0 || dashboard.stats.pending > 0) ? (
              <button
                onClick={() => void runAction('pause', () => invoke('pause_video_queue'), '队列已暂停。')}
                className="px-4 py-2.5 rounded-lg border border-amber-500/30 bg-amber-500/10 text-amber-300 text-sm font-semibold flex items-center gap-2"
              >
                <Pause className="w-4 h-4" />
                暂停队列
              </button>
            ) : (
              <button
                onClick={() => void runAction('start', async () => {
                  await invoke('start_video_queue');
                  await invoke('tick_video_queue');
                }, '队列已开始运行。')}
                disabled={dashboard.stats.pending === 0}
                className="px-4 py-2.5 rounded-lg bg-emerald-600 hover:bg-emerald-500 disabled:opacity-40 text-white text-sm font-semibold flex items-center gap-2"
                title={dashboard.stats.pending === 0 ? '所有课程已全部完成' : '开始自动播放待播放课程'}
              >
                <Play className="w-4 h-4 fill-current" />
                开始队列
              </button>
            )}
          </div>
        </div>
        {activeTasks.length > 0 && (
          <div className="mt-4 pt-4 border-t th-border flex flex-wrap items-center gap-3">
            <span className="text-xs font-semibold th-text-muted flex items-center gap-2 shrink-0">
              <span className="relative flex h-2 w-2">
                <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-emerald-400 opacity-75" />
                <span className="relative inline-flex rounded-full h-2 w-2 bg-emerald-500" />
              </span>
              当前正在播放：
            </span>
            {activeTasks.map(({ topicId, topicTitle, course }) => (
              <button
                key={course.id}
                onClick={() => {
                  setExpandedTopics((prev) => new Set(prev).add(topicId));
                  document.getElementById('topic-' + topicId)?.scrollIntoView({ behavior: 'smooth', block: 'center' });
                }}
                className="px-3 py-1.5 rounded-lg bg-indigo-500/10 border border-indigo-500/30 text-xs flex items-center gap-2 hover:bg-indigo-500/20 hover:border-indigo-500/50 transition-colors text-left"
                title="点击展开并定位到该专题"
              >
                <span className="text-indigo-400 font-semibold">【{topicTitle}】</span>
                <span className="th-text-2 font-medium truncate max-w-xs">{course.title}</span>
                <span className="text-emerald-400 font-bold tabular-nums">{Math.round(course.progress)}%</span>
              </button>
            ))}
          </div>
        )}
      </section>

      <section className="mb-5 rounded-xl border border-orange-500/25 bg-orange-500/10 px-5 py-4">
        <div className="flex items-start gap-3">
          <ShieldCheck className="w-5 h-5 text-orange-400 shrink-0 mt-0.5" />
          <div>
            <h3 className="text-sm font-semibold text-orange-300">本人处理边界</h3>
            <p className="text-xs text-orange-200/70 mt-1 leading-relaxed">
              考试不会自动答题；翻页课件与知识材料本期不会自动点击。它们会保留在专题进度中，完成后使用“同步专题”读取平台的“已考试 / 已完成 / 100%”状态。
            </p>
          </div>
        </div>
      </section>

      <section className="th-bg-card border th-border rounded-xl overflow-hidden shadow-xl">
        <div className="px-5 py-4 border-b th-border flex items-center justify-between th-bg-surface-h">
          <div className="flex items-center gap-2">
            <BookOpen className="w-4 h-4 text-indigo-400" />
            <h2 className="text-sm font-semibold th-text-2">已导入专题</h2>
          </div>
          <span className="text-xs th-text-muted">{dashboard.topics.length} 个专题</span>
        </div>

        {dashboard.topics.length === 0 ? (
          <div className="py-16 text-center px-6">
            <BookOpen className="w-10 h-10 th-text-ghost mx-auto mb-3" />
            <p className="text-sm th-text-3">还没有导入专题</p>
            <p className="text-xs th-text-muted mt-2">先登录平台，进入专题课程列表页，然后点击“导入当前专题”。</p>
          </div>
        ) : (
          <div className="divide-y th-divide">
            {dashboard.topics.map((topic) => {
              const expanded = expandedTopics.has(topic.id);
              const providerName = topic.provider === 'ulearn' ? '银联乐学' : 'YS学堂';
              const denominator = topic.totalCount || topic.courses.length;
              const numerator = topic.completedCount || topic.courses.filter((course) => course.status === 'completed').length;
              const activeCourse = topic.courses.find((course) => course.status === 'opening' || course.status === 'playing' || course.status === 'verifying');
              return (
                <div
                  key={topic.id}
                  id={'topic-' + topic.id}
                  className={cx(
                    'transition-colors duration-200',
                    activeCourse && 'bg-indigo-500/[0.04]'
                  )}
                >
                  <div className="px-5 py-4 flex items-center gap-4 th-hover-surface">
                    <button
                      onClick={() => toggleTopic(topic.id)}
                      className="p-1 rounded-md th-text-muted hover:text-indigo-400 th-hover-surface shrink-0"
                      aria-label={expanded ? '收起专题' : '展开专题'}
                    >
                      <ChevronRight className={cx('w-4 h-4 transition-transform duration-200', expanded && 'rotate-90 text-indigo-400')} />
                    </button>
                    <div
                      onClick={() => toggleTopic(topic.id)}
                      className="min-w-0 flex-1 cursor-pointer"
                    >
                      <div className="flex flex-wrap items-center gap-2">
                        <span className="text-[11px] px-2 py-0.5 rounded bg-indigo-500/10 text-indigo-400 border border-indigo-500/20">
                          {providerName}
                        </span>
                        <h3 className="text-sm font-semibold th-text-2 truncate">{topic.title}</h3>
                        {activeCourse && (
                          <span className="text-[11px] px-2.5 py-0.5 rounded-full bg-emerald-500/10 text-emerald-400 border border-emerald-500/30 font-medium flex items-center gap-1.5 animate-pulse">
                            <Loader2 className="w-3 h-3 animate-spin" />
                            正在播放: {activeCourse.title} ({Math.round(activeCourse.progress)}%)
                          </span>
                        )}
                      </div>
                      <div className="mt-2 flex items-center gap-3">
                        <div className="h-1.5 rounded-full th-bg-surface flex-1 max-w-sm overflow-hidden">
                          <div className="h-full bg-emerald-500 rounded-full" style={{ width: String(Math.min(100, topic.progress)) + '%' }} />
                        </div>
                        <span className="text-xs th-text-muted tabular-nums">
                          {numerator}/{denominator} · {Math.round(topic.progress)}%
                        </span>
                        <span className="text-[11px] th-text-faint hidden lg:inline">同步于 {formatSyncTime(topic.lastSyncedAt)}</span>
                      </div>
                    </div>
                    <div className="flex items-center gap-1 shrink-0">
                      <button
                        onClick={() => void runAction(
                          'reset-' + topic.id,
                          () => invoke('reset_video_topic', { topicId: topic.id }),
                          '专题所有课程已重置为待播放状态。',
                        )}
                        disabled={busyKey === 'reset-' + topic.id}
                        className="p-2 rounded-lg th-text-muted hover:text-amber-400 th-hover-surface"
                        title="重置本专题（重新设为待播放）"
                      >
                        <RotateCcw className={cx('w-4 h-4', busyKey === 'reset-' + topic.id && 'animate-spin')} />
                      </button>
                      <button
                        onClick={() => void syncTopic(topic)}
                        className="p-2 rounded-lg th-text-muted hover:text-indigo-400 th-hover-surface"
                        title="同步专题"
                      >
                        <RefreshCw className={cx('w-4 h-4', busyKey === 'sync-' + topic.id && 'animate-spin')} />
                      </button>
                      <button
                        onClick={() => setDeleteConfirmTopic(topic)}
                        className="p-2 rounded-lg th-text-muted hover:text-rose-400 hover:bg-rose-500/10"
                        title="从管理台移除"
                      >
                        <Trash2 className="w-4 h-4" />
                      </button>
                    </div>
                  </div>

                  {expanded && (
                    <div className="border-t th-border bg-black/5">
                      {topic.courses.map((course) => {
                        const kindKey = resolveCourseKind(course);
                        const kind = KIND_META[kindKey] || KIND_META.video;
                        const status = STATUS_META[course.status] || STATUS_META.pending;
                        const KindIcon = kind.icon;
                        const isLoginError = course.status === 'attention' && Boolean(course.lastError?.includes('登录'));
                        const isSkipped = course.status === 'attention' && Boolean(course.lastError?.includes('跳过'));
                        const statusLabel = isLoginError
                          ? '需要登录'
                          : isSkipped
                          ? '已自动跳过'
                          : (course.status === 'playing' ? '正在播放 ' + Math.round(course.progress) + '%' : status.label);
                        const statusClasses = isLoginError
                          ? 'text-amber-400 bg-amber-500/10 border-amber-500/30'
                          : isSkipped
                          ? 'text-amber-400 bg-amber-500/10 border-amber-500/30'
                          : status.classes;
                        return (
                          <div key={course.id} className="px-6 py-3.5 ml-7 border-b last:border-b-0 th-border flex items-center gap-4">
                            <div className={cx('w-9 h-9 rounded-lg flex items-center justify-center shrink-0 border', kind.classes)} title={kind.label}>
                              <KindIcon className="w-4 h-4" />
                            </div>
                            <div className="min-w-0 flex-1">
                              <div className="flex flex-wrap items-center gap-2">
                                <span className="text-sm font-medium th-text-2 truncate">{course.title}</span>
                                <span className={cx('text-[10px] px-1.5 py-0.5 rounded border flex items-center gap-1', statusClasses)}>
                                  {(course.status === 'opening' || course.status === 'playing') && <Loader2 className="w-2.5 h-2.5 animate-spin" />}
                                  {statusLabel}
                                </span>
                                <span className={cx('text-[10px] px-1.5 py-0.5 rounded border font-medium', kind.badgeClasses)}>
                                  {kind.label}
                                </span>
                              </div>
                              <div className="flex flex-wrap items-center gap-3 mt-1.5 text-[11px] th-text-muted">
                                {course.sectionTitle && <span>{course.sectionTitle}</span>}
                                <span className="flex items-center gap-1">
                                  <Clock3 className="w-3 h-3" />
                                  {formatDuration(course.durationSeconds)}
                                </span>
                                {course.status === 'playing' ? (
                                  <div className="flex items-center gap-2">
                                    <div className="w-20 h-1.5 rounded-full th-bg-surface overflow-hidden">
                                      <div
                                        className="h-full bg-indigo-500 rounded-full transition-all duration-300"
                                        style={{ width: String(Math.max(4, course.progress)) + '%' }}
                                      />
                                    </div>
                                    <span className="text-indigo-400 font-medium">{Math.round(course.progress)}%</span>
                                  </div>
                                ) : (
                                  <span>平台进度 {Math.round(course.progress)}%</span>
                                )}
                                {course.status === 'attention' && course.lastError && (
                                   <span className={cx('flex items-center gap-1', (isLoginError || isSkipped) ? 'text-amber-400 font-medium' : 'text-rose-400')}>
                                    <AlertTriangle className="w-3 h-3" />{course.lastError}
                                  </span>
                                )}
                              </div>
                            </div>
                            <div className="flex items-center gap-2 shrink-0">
                              {/* 仅正在播放中的视频，需要人工处理的考试/课件，或者需要登录/处理的异常课程，展示打开按钮 */}
                              {(course.status === 'opening' || course.status === 'playing' || course.status === 'attention' || kindKey !== 'video') && (
                                <button
                                  onClick={() => void runAction(
                                    'open-course-' + course.id,
                                    () => invoke('open_video_course', { courseId: course.id }),
                                  )}
                                  className={cx(
                                    'px-3 py-1.5 rounded-md border text-xs font-semibold flex items-center gap-1.5 th-hover-surface',
                                    isLoginError
                                      ? 'border-amber-500/30 bg-amber-500/10 text-amber-300 hover:bg-amber-500/20'
                                      : 'th-border-subtle th-bg-input-alt th-text-2'
                                  )}
                                  title={isLoginError ? '打开窗口完成扫码/账号登录' : '打开网页窗口查看内容'}
                                >
                                  <ExternalLink className="w-3.5 h-3.5" />
                                  {isLoginError ? '打开登录' : kindKey === 'exam' ? '打开考试' : kindKey === 'slides' ? '打开课件' : kindKey === 'material' ? '打开资料' : '打开内容'}
                                </button>
                              )}
                              {(course.status === 'opening' || course.status === 'playing') && (
                                <button
                                  onClick={() => void runAction(
                                    'pause-course-' + course.id,
                                    () => invoke('pause_video_course', { courseId: course.id }),
                                    '已跳过当前课程，开始播放下一门。',
                                  )}
                                  disabled={busyKey === 'pause-course-' + course.id}
                                  className="px-3 py-1.5 rounded-md border border-amber-500/30 bg-amber-500/10 text-amber-300 text-xs font-semibold flex items-center gap-1.5 hover:bg-amber-500/20 disabled:opacity-50"
                                  title="跳过当前视频，开始播放下一个"
                                >
                                  {busyKey === 'pause-course-' + course.id
                                    ? <Loader2 className="w-3.5 h-3.5 animate-spin" />
                                    : <SkipForward className="w-3.5 h-3.5" />}
                                  跳过
                                </button>
                              )}
                              {course.status === 'attention' && (kindKey === 'video' || kindKey === 'slides') && (
                                <button
                                  onClick={() => void runAction(
                                    'retry-' + course.id,
                                    () => invoke('retry_video_course', { courseId: course.id }),
                                    '已重新加入待播放队列，当前视频播放完毕后将自动播放。',
                                  )}
                                  className="px-3 py-1.5 rounded-md border border-indigo-500/25 bg-indigo-500/10 text-indigo-400 text-xs font-semibold"
                                >
                                  重试
                                </button>
                              )}
                              {(course.status === 'opening' || course.status === 'playing') && <Loader2 className="w-4 h-4 animate-spin text-indigo-400" />}
                              {course.status === 'completed' && (
                                <div className="flex items-center gap-1.5">
                                  <button
                                    onClick={() => void runAction(
                                      'reset-course-' + course.id,
                                      () => invoke('reset_video_course', { courseId: course.id }),
                                      '已将该课程重置为待播放。',
                                    )}
                                    disabled={busyKey === 'reset-course-' + course.id}
                                    className="p-1.5 rounded-md th-text-muted hover:text-amber-400 th-hover-surface"
                                    title="重新学习（重置为待播放）"
                                  >
                                    <RotateCcw className={cx('w-3.5 h-3.5', busyKey === 'reset-course-' + course.id && 'animate-spin')} />
                                  </button>
                                  <CheckCircle2 className="w-5 h-5 text-emerald-400" />
                                </div>
                              )}
                            </div>
                          </div>
                        );
                      })}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </section>

      {deleteConfirmTopic && (
        <div className="fixed inset-0 bg-black/60 backdrop-blur-sm z-50 flex items-center justify-center p-4">
          <div className="th-bg-card border th-border rounded-xl max-w-md w-full p-6 shadow-2xl animate-in fade-in zoom-in-95 duration-150">
            <h3 className="text-base font-bold th-text">移除专题确认</h3>
            <p className="text-sm th-text-muted mt-2 leading-relaxed">
              确定从管理台移除专题“<span className="th-text font-medium">{deleteConfirmTopic.title}</span>”吗？
            </p>
            <p className="text-xs th-text-faint mt-1">
              注意：仅从本工具管理台移除，平台的学习记录不会被删除。
            </p>
            <div className="mt-6 flex justify-end gap-3">
              <button
                onClick={() => setDeleteConfirmTopic(null)}
                className="px-4 py-2 rounded-lg border th-border th-bg-surface th-text-2 text-xs font-semibold th-hover-surface"
              >
                取消
              </button>
              <button
                onClick={() => {
                  const topicId = deleteConfirmTopic.id;
                  setDeleteConfirmTopic(null);
                  void runAction(
                    'remove-topic',
                    () => invoke('remove_video_topic', { topicId }),
                    '专题已从管理台移除。',
                  );
                }}
                className="px-4 py-2 rounded-lg bg-rose-600 hover:bg-rose-500 text-white text-xs font-semibold"
              >
                确认移除
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
