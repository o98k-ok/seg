<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { onMount } from 'svelte';
  import type { Session, Source } from '$lib/types';

  type SortMode = 'duration' | 'recent';
  type StatusTab = 'running' | 'finished';
  type Tab = 'all' | Source | StatusTab;
  type Limit = 20 | 50 | 100 | 0;
  type RefreshMs = 0 | 30_000 | 60_000 | 300_000 | 600_000;
  type Theme = 'dark' | 'light';

  interface Settings {
    sortMode: SortMode;
    limit: Limit;
    refreshIntervalMs: RefreshMs;
    codexHome: string;
    claudeHome: string;
    theme: Theme;
  }

  const DEFAULT_SETTINGS: Settings = {
    sortMode: 'duration',
    limit: 50,
    refreshIntervalMs: 300_000,
    codexHome: '',
    claudeHome: '',
    theme: 'dark',
  };

  const STORAGE_KEY = 'seg:settings:v1';

  function loadSettings(): Settings {
    if (typeof localStorage === 'undefined') return { ...DEFAULT_SETTINGS };
    try {
      const raw = localStorage.getItem(STORAGE_KEY);
      if (!raw) return { ...DEFAULT_SETTINGS };
      const parsed = JSON.parse(raw) as Partial<Settings>;
      return { ...DEFAULT_SETTINGS, ...parsed };
    } catch {
      return { ...DEFAULT_SETTINGS };
    }
  }

  let sessions = $state<Session[]>([]);
  let loading = $state(true);
  let tab = $state<Tab>('all');
  let now = $state(Date.now());

  let settings = $state<Settings>(loadSettings());
  let settingsOpen = $state(false);
  let selectedProjects = $state<string[]>([]);
  let selectedModes = $state<string[]>([]);

  async function refresh() {
    loading = true;
    try {
      sessions = await invoke<Session[]>('list_sessions', {
        codexHome: settings.codexHome.trim() || null,
        claudeHome: settings.claudeHome.trim() || null,
      });
    } catch (err) {
      console.error('Failed to refresh sessions', err);
    } finally {
      loading = false;
    }
  }

  async function quitApp() {
    await invoke('quit_app');
  }

  onMount(() => {
    refresh();
    const tick = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(tick);
  });

  $effect(() => {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
    } catch {
      /* storage quota or disabled */
    }
  });

  $effect(() => {
    const ms = settings.refreshIntervalMs;
    if (!ms) return;
    const id = setInterval(refresh, ms);
    return () => clearInterval(id);
  });

  const STATUS_RANK: Record<Session['status'], number> = {
    running: 0,
    finished: 1,
    stopped: 2,
  };

  function projectName(p: string | null): { name: string; full: string } {
    if (!p) return { name: '—', full: '' };
    const trimmed = p.replace(/\/+$/, '');
    const name = trimmed.split('/').filter(Boolean).at(-1) ?? trimmed;
    return { name, full: p };
  }

  function meaningfulMode(s: Session): string | null {
    const mode = s.display_segment.mode;
    if (!mode) return null;
    if (s.source === 'codex' && mode === 'default') return null;
    if (s.source === 'claude' && (mode === 'default' || mode === 'bypassPermissions')) {
      return null;
    }
    return mode;
  }

  const tabSessions = $derived.by(() => {
    if (tab === 'all') return sessions;
    if (tab === 'running' || tab === 'finished') {
      return sessions.filter((s) => s.status === tab);
    }
    return sessions.filter((s) => s.source === tab);
  });

  const projectTags = $derived.by(() => {
    const counts = new Map<string, number>();
    for (const s of tabSessions) {
      if (!s.cwd) continue;
      const name = projectName(s.cwd).name;
      if (!name || name === '—') continue;
      counts.set(name, (counts.get(name) ?? 0) + 1);
    }
    return [...counts.entries()]
      .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
      .map(([name, count]) => ({ name, count }));
  });

  const modeTags = $derived.by(() => {
    const counts = new Map<string, number>();
    for (const s of tabSessions) {
      const m = meaningfulMode(s);
      if (m) counts.set(m, (counts.get(m) ?? 0) + 1);
      if (s.has_plan_mode) counts.set('plan', (counts.get('plan') ?? 0) + 1);
    }
    return [...counts.entries()]
      .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
      .map(([name, count]) => ({ name, count }));
  });

  const filtered = $derived.by(() => {
    let arr: Session[] = tabSessions;
    if (selectedProjects.length > 0) {
      arr = arr.filter((s) => {
        if (!s.cwd) return false;
        return selectedProjects.includes(projectName(s.cwd).name);
      });
    }
    if (selectedModes.length > 0) {
      arr = arr.filter((s) => {
        const m = meaningfulMode(s);
        if (m && selectedModes.includes(m)) return true;
        if (s.has_plan_mode && selectedModes.includes('plan')) return true;
        return false;
      });
    }
    arr = [...arr];
    const secondary =
      settings.sortMode === 'duration'
        ? (a: Session, b: Session) =>
            b.display_segment.duration_secs - a.display_segment.duration_secs
        : (a: Session, b: Session) =>
            new Date(b.last_event_at).getTime() - new Date(a.last_event_at).getTime();
    arr.sort((a, b) => {
      const r = STATUS_RANK[a.status] - STATUS_RANK[b.status];
      return r !== 0 ? r : secondary(a, b);
    });
    if (settings.limit > 0) arr = arr.slice(0, settings.limit);
    return arr;
  });

  const runningCount = $derived(sessions.filter((s) => s.status === 'running').length);
  const finishedCount = $derived(sessions.filter((s) => s.status === 'finished').length);
  const codexCount = $derived(sessions.filter((s) => s.source === 'codex').length);
  const claudeCount = $derived(sessions.filter((s) => s.source === 'claude').length);
  const hasFilters = $derived(selectedProjects.length + selectedModes.length > 0);

  function toggleProject(name: string) {
    selectedProjects = selectedProjects.includes(name)
      ? selectedProjects.filter((n) => n !== name)
      : [...selectedProjects, name];
  }
  function toggleMode(name: string) {
    selectedModes = selectedModes.includes(name)
      ? selectedModes.filter((n) => n !== name)
      : [...selectedModes, name];
  }
  function clearFilters() {
    selectedProjects = [];
    selectedModes = [];
  }

  function formatDuration(secs: number): string {
    if (secs < 1) return '0s';
    if (secs < 60) return `${secs}s`;
    const m = Math.floor(secs / 60);
    if (m < 60) {
      const s = secs % 60;
      return s === 0 ? `${m}m` : `${m}m ${s}s`;
    }
    const h = Math.floor(m / 60);
    const remM = m % 60;
    return remM === 0 ? `${h}h` : `${h}h ${remM}m`;
  }

  function barSegLayout(s: Session, seg: import('$lib/types').Segment) {
    const t0 = new Date(s.started_at).getTime();
    const t1 = new Date(s.last_event_at).getTime();
    const total = Math.max(t1 - t0, 1);
    const segStart = new Date(seg.start).getTime();
    const segEnd = new Date(seg.end).getTime();
    const isDisplay =
      seg.start === s.display_segment.start && seg.end === s.display_segment.end;
    const left = Math.max(0, ((segStart - t0) / total) * 100);
    const rawWidth = ((segEnd - segStart) / total) * 100;
    const width = Math.min(100 - left, Math.max(rawWidth, 1.6));
    return { left, width, isDisplay };
  }

  function relTime(iso: string, nowMs: number): string {
    const ms = nowMs - new Date(iso).getTime();
    if (ms < 0) return 'now';
    const s = Math.floor(ms / 1000);
    if (s < 5) return 'now';
    if (s < 60) return `${s}s ago`;
    const m = Math.floor(s / 60);
    if (m < 60) return `${m}m ago`;
    const h = Math.floor(m / 60);
    if (h < 24) return `${h}h ago`;
    const d = Math.floor(h / 24);
    if (d < 30) return `${d}d ago`;
    const mo = Math.floor(d / 30);
    if (mo < 12) return `${mo}mo ago`;
    return `${Math.floor(mo / 12)}y ago`;
  }

  const LIMIT_OPTIONS: { value: Limit; label: string }[] = [
    { value: 20, label: '20' },
    { value: 50, label: '50' },
    { value: 100, label: '100' },
    { value: 0, label: 'All' },
  ];
  const REFRESH_OPTIONS: { value: RefreshMs; label: string }[] = [
    { value: 0, label: 'Off' },
    { value: 30_000, label: '30s' },
    { value: 60_000, label: '1m' },
    { value: 300_000, label: '5m' },
    { value: 600_000, label: '10m' },
  ];
</script>

<div class="shell" data-theme={settings.theme}>
  <header class="topbar">
    <nav class="tabs">
      <button class:active={tab === 'all'} onclick={() => (tab = 'all')}>
        all <span class="tab-count">{sessions.length}</span>
      </button>
      <button class:active={tab === 'codex'} onclick={() => (tab = 'codex')}>
        codex <span class="tab-count">{codexCount}</span>
      </button>
      <button class:active={tab === 'claude'} onclick={() => (tab = 'claude')}>
        claude <span class="tab-count">{claudeCount}</span>
      </button>
      <span class="tab-divider" aria-hidden="true"></span>
      <button
        class="status-tab status-running"
        class:active={tab === 'running'}
        onclick={() => (tab = 'running')}
      >
        <span class="status-dot"></span>
        running <span class="tab-count">{runningCount}</span>
      </button>
      <button
        class="status-tab status-finished"
        class:active={tab === 'finished'}
        onclick={() => (tab = 'finished')}
      >
        <span class="status-dot"></span>
        finished <span class="tab-count">{finishedCount}</span>
      </button>
    </nav>
  </header>

  {#if settingsOpen}
    <div class="filter-row settings-spacer"></div>
  {:else}
    <div class="filter-row">
      {#if projectTags.length === 0 && modeTags.length === 0}
        <span class="filter-empty">no tags yet</span>
      {:else}
        <div class="chips" role="group" aria-label="Filters">
          {#each projectTags as t (`p:${t.name}`)}
            <button
              class="filter-chip type-project"
              class:on={selectedProjects.includes(t.name)}
              onclick={() => toggleProject(t.name)}
              title="{t.count} session{t.count > 1 ? 's' : ''}"
            >
              <span class="chip-label">{t.name}</span>
              <span class="chip-count">{t.count}</span>
            </button>
          {/each}
          {#each modeTags as t (`m:${t.name}`)}
            <button
              class="filter-chip type-mode mode-{t.name}"
              class:on={selectedModes.includes(t.name)}
              onclick={() => toggleMode(t.name)}
              title="{t.count} session{t.count > 1 ? 's' : ''}"
            >
              <span class="chip-dot"></span>
              <span class="chip-label">{t.name}</span>
              <span class="chip-count">{t.count}</span>
            </button>
          {/each}
          {#if hasFilters}
            <button class="filter-chip clear-chip" onclick={clearFilters} title="Clear filters">
              clear ×
            </button>
          {/if}
        </div>
      {/if}
    </div>
  {/if}

  <main class="list" class:no-scroll={settingsOpen}>
    {#if settingsOpen}
      <section class="settings-panel" aria-label="Settings">
        <div class="settings-row">
          <span class="settings-label">Theme</span>
          <div class="seg">
            <button
              class:active={settings.theme === 'dark'}
              onclick={() => (settings.theme = 'dark')}
            >Dark</button>
            <button
              class:active={settings.theme === 'light'}
              onclick={() => (settings.theme = 'light')}
            >Light</button>
          </div>
        </div>
        <div class="settings-row">
          <span class="settings-label">Sort</span>
          <div class="seg">
            <button
              class:active={settings.sortMode === 'duration'}
              onclick={() => (settings.sortMode = 'duration')}
            >Duration</button>
            <button
              class:active={settings.sortMode === 'recent'}
              onclick={() => (settings.sortMode = 'recent')}
            >Recent</button>
          </div>
        </div>
        <div class="settings-row">
          <span class="settings-label">Show</span>
          <div class="seg">
            {#each LIMIT_OPTIONS as opt (opt.value)}
              <button
                class:active={settings.limit === opt.value}
                onclick={() => (settings.limit = opt.value)}
              >{opt.label}</button>
            {/each}
          </div>
        </div>
        <div class="settings-row">
          <span class="settings-label">Refresh</span>
          <div class="seg">
            {#each REFRESH_OPTIONS as opt (opt.value)}
              <button
                class:active={settings.refreshIntervalMs === opt.value}
                onclick={() => (settings.refreshIntervalMs = opt.value)}
              >{opt.label}</button>
            {/each}
          </div>
        </div>
        <div class="settings-row">
          <span class="settings-label">Codex</span>
          <input
            class="path-input"
            type="text"
            spellcheck="false"
            autocomplete="off"
            placeholder="~/.codex"
            bind:value={settings.codexHome}
            onchange={refresh}
          />
        </div>
        <div class="settings-row">
          <span class="settings-label">Claude</span>
          <input
            class="path-input"
            type="text"
            spellcheck="false"
            autocomplete="off"
            placeholder="~/.claude"
            bind:value={settings.claudeHome}
            onchange={refresh}
          />
        </div>
        <p class="settings-hint">
          Scans <code>{settings.codexHome.trim() || '~/.codex'}/sessions</code> and
          <code>{settings.claudeHome.trim() || '~/.claude'}/projects</code>.
          Leave blank for defaults.
        </p>
        <div class="settings-actions">
          <button class="quit-btn" onclick={quitApp} title="Quit seg">
            <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4" />
              <polyline points="16 17 21 12 16 7" />
              <line x1="21" y1="12" x2="9" y2="12" />
            </svg>
            Quit seg
          </button>
        </div>
      </section>
    {:else if loading && sessions.length === 0}
      {#each Array(4) as _, i (i)}
        <div class="skeleton" style="animation-delay: {i * 120}ms"></div>
      {/each}
    {:else if filtered.length === 0}
      <div class="empty">
        {#if sessions.length === 0}
          <span class="empty-line"><code>~/.codex/sessions</code> is empty.</span>
          <span class="empty-line dim">Run a Codex session and reopen.</span>
        {:else if hasFilters}
          <span class="empty-line">No sessions match the selected tags.</span>
          <button class="empty-action" onclick={clearFilters}>Clear filters</button>
        {:else}
          <span class="empty-line">No sessions in this view.</span>
        {/if}
      </div>
    {:else}
      {#each filtered as s, i (s.file_path)}
        {@const path = projectName(s.cwd)}
        {@const mode = meaningfulMode(s)}
        <article
          class="card status-{s.status}"
          style="animation-delay: {Math.min(i, 11) * 50}ms"
        >
          <div class="row top">
            <div class="identity">
              <span class="dot status-{s.status}"></span>
              <span class="cwd" title={path.full}>
                {path.name}
              </span>
              <div class="meta">
                <span class="chip source-{s.source}">{s.source}</span>
                {#if s.has_plan_mode}
                  <span class="chip mode-plan" title="/plan appeared in this session">PLAN</span>
                {/if}
                {#if s.goal}
                  <span class="chip goal-{s.goal.status ?? 'unknown'}" title={s.goal.objective}>goal</span>
                {:else if mode && mode !== 'plan'}
                  <span class="chip mode-{mode}">{mode}</span>
                {/if}
                {#if s.model}<span class="meta-item">{s.model}</span>{/if}
                {#if s.reasoning_effort}<span class="meta-item">{s.reasoning_effort}</span>{/if}
              </div>
            </div>
            <span class="rel">{relTime(s.last_event_at, now)}</span>
          </div>

          <div class="row bar">
            <div class="bar-track">
              {#each s.segments as seg, segI (segI)}
                {@const layout = barSegLayout(s, seg)}
                <button
                  type="button"
                  class="bar-seg"
                  class:is-display={layout.isDisplay}
                  style="left: {layout.left}%; width: {layout.width}%"
                  data-duration={formatDuration(seg.duration_secs)}
                  aria-label="Segment duration {formatDuration(seg.duration_secs)}"
                ></button>
              {/each}
              {#if s.status === 'running'}
                <span class="bar-live"></span>
              {/if}
            </div>
          </div>
        </article>
      {/each}
    {/if}
  </main>

  <footer class="statusbar">
    <span>
      <span class="count">{sessions.length}</span> sessions
      {#if runningCount > 0}
        <span class="sep">·</span>
        <span class="running">{runningCount} running</span>
      {/if}
      {#if !settingsOpen && (hasFilters || (settings.limit > 0 && sessions.length > settings.limit))}
        <span class="sep">·</span>
        <span class="filtered">{filtered.length} shown</span>
      {/if}
    </span>
    <div class="statusbar-actions">
      <button
        class="icon-btn gear"
        class:active={settingsOpen}
        onclick={() => (settingsOpen = !settingsOpen)}
        aria-label="Settings"
        title="Settings"
      >
        <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="3" />
          <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
        </svg>
      </button>
      <button
        class="icon-btn refresh"
        class:loading
        onclick={refresh}
        title="Rescan"
        aria-label={loading ? 'Refreshing sessions' : 'Rescan sessions'}
        disabled={loading}
      >↻</button>
    </div>
  </footer>
</div>

<style>
  :global(html), :global(body) {
    background: transparent;
    font-family:
      -apple-system, BlinkMacSystemFont, 'SF Pro Text', 'SF Pro Display',
      system-ui, sans-serif;
  }

  .shell {
    position: fixed;
    inset: 0;
    background: var(--bg);
    border-radius: 16px;
    display: grid;
    grid-template-rows: 38px auto 1fr 32px;
    grid-template-columns: minmax(0, 1fr);
    overflow: hidden;
    color: var(--text);
    box-shadow:
      inset 0 0 0 0.5px var(--border-soft),
      0 24px 48px -12px rgb(0 0 0 / 0.75),
      0 0 0 1px rgb(0 0 0 / 0.5);
    transition: background 0.25s ease, color 0.25s ease;
  }

  /* ── theme tokens (Apple Health-ish) ─────── */
  .shell[data-theme='dark'] {
    --bg: oklch(0.14 0 0);
    --card: oklch(0.21 0 0);
    --card-hover: oklch(0.24 0 0);
    --border-soft: oklch(1 0 0 / 0.06);

    --text: oklch(0.97 0 0);
    --text-dim: oklch(0.66 0 0);
    --text-faint: oklch(0.42 0 0);
    --text-mute: oklch(0.32 0 0);

    --accent-green: oklch(0.82 0.18 145);
    --accent-green-dim: oklch(0.45 0.09 145);
    --accent-blue: oklch(0.7 0.16 245);
    --accent-blue-dim: oklch(0.42 0.08 245);
    --accent-yellow: oklch(0.82 0.13 95);
    --accent-yellow-dim: oklch(0.5 0.08 95);
    --accent-red: oklch(0.72 0.18 28);
    --accent-red-dim: oklch(0.5 0.13 28);

    --overlay-soft: oklch(1 0 0 / 0.06);
    --overlay: oklch(1 0 0 / 0.08);
    --overlay-medium: oklch(1 0 0 / 0.12);
    --overlay-border: oklch(1 0 0 / 0.1);
    --neutral-mid: oklch(0.55 0 0);

    --tooltip-bg: oklch(0.12 0 0 / 0.96);
    --tooltip-text: oklch(0.97 0 0);
    --tooltip-border: oklch(1 0 0 / 0.1);
  }

  .shell[data-theme='light'] {
    --bg: oklch(0.98 0 0);
    --card: oklch(0.94 0 0);
    --card-hover: oklch(0.91 0 0);
    --border-soft: oklch(0 0 0 / 0.06);

    --text: oklch(0.18 0 0);
    --text-dim: oklch(0.4 0 0);
    --text-faint: oklch(0.55 0 0);
    --text-mute: oklch(0.7 0 0);

    --accent-green: oklch(0.58 0.16 145);
    --accent-green-dim: oklch(0.78 0.08 145);
    --accent-blue: oklch(0.55 0.16 245);
    --accent-blue-dim: oklch(0.78 0.06 245);
    --accent-yellow: oklch(0.68 0.14 75);
    --accent-yellow-dim: oklch(0.82 0.07 80);
    --accent-red: oklch(0.56 0.18 28);
    --accent-red-dim: oklch(0.72 0.1 28);

    --overlay-soft: oklch(0 0 0 / 0.05);
    --overlay: oklch(0 0 0 / 0.07);
    --overlay-medium: oklch(0 0 0 / 0.1);
    --overlay-border: oklch(0 0 0 / 0.1);
    --neutral-mid: oklch(0.6 0 0);

    --tooltip-bg: oklch(0.18 0 0 / 0.96);
    --tooltip-text: oklch(0.97 0 0);
    --tooltip-border: oklch(1 0 0 / 0.12);
  }

  /* ── top bar (tabs + gear) ───────────────── */
  .topbar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 10px;
    background: var(--bg);
    min-width: 0;
  }
  .tabs {
    display: flex;
    align-items: center;
    gap: 2px;
    flex: 1;
    min-width: 0;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .tabs::-webkit-scrollbar {
    display: none;
  }
  .tabs button {
    background: transparent;
    border: none;
    color: var(--text-faint);
    cursor: pointer;
    font: inherit;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.01em;
    padding: 5px 10px;
    border-radius: 6px;
    transition: color 0.15s, background 0.15s;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }
  .tabs button:hover {
    color: var(--text-dim);
  }
  .tabs button.active {
    color: var(--text);
    background: var(--card);
  }
  .tabs button.active .tab-count {
    color: var(--accent-green);
  }
  .tab-count {
    font-variant-numeric: tabular-nums;
    color: var(--text-mute);
    font-weight: 700;
    font-size: 10px;
  }
  .tab-divider {
    width: 1px;
    height: 12px;
    background: var(--border-soft);
    margin: 0 4px;
    flex-shrink: 0;
  }
  .status-tab .status-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex-shrink: 0;
    transition: box-shadow 0.2s ease;
  }
  .status-tab.status-running .status-dot {
    background: var(--accent-yellow);
  }
  .status-tab.status-running.active .status-dot {
    box-shadow: 0 0 0 3px color-mix(in oklch, var(--accent-yellow) 18%, transparent);
    animation: ring 2.2s ease-in-out infinite;
  }
  .status-tab.status-running.active {
    color: var(--accent-yellow);
  }
  .status-tab.status-running.active .tab-count {
    color: var(--accent-yellow);
  }
  .status-tab.status-finished .status-dot {
    background: var(--accent-green);
  }
  .status-tab.status-finished.active .status-dot {
    box-shadow: 0 0 0 3px color-mix(in oklch, var(--accent-green) 18%, transparent);
  }
  .status-tab.status-finished.active {
    color: var(--accent-green);
  }
  .status-tab.status-finished.active .tab-count {
    color: var(--accent-green);
  }
  .icon-btn {
    width: 20px;
    height: 20px;
    background: var(--card);
    border: none;
    border-radius: 6px;
    color: var(--text-dim);
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    padding: 0;
    transition:
      transform 0.32s cubic-bezier(0.2, 0.8, 0.2, 1),
      background 0.15s,
      color 0.15s;
  }
  .icon-btn:hover:not(:disabled) {
    background: var(--card-hover);
    color: var(--text);
  }
  .icon-btn:active:not(:disabled) {
    transform: scale(0.88);
  }
  .icon-btn:disabled {
    cursor: default;
    opacity: 0.72;
  }
  .refresh {
    font-size: 15px;
    font-weight: 700;
    line-height: 1;
  }
  .refresh.loading {
    animation: spin 0.9s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .gear.active {
    background: var(--card-hover);
    color: var(--accent-green);
    transform: rotate(60deg);
  }
  .gear.active:active {
    transform: rotate(60deg) scale(0.88);
  }

  /* ── filter row (replaces search) ────────── */
  .filter-row {
    display: flex;
    align-items: flex-start;
    padding: 0 10px;
    background: var(--bg);
    max-height: 60px;
    overflow-y: auto;
    overflow-x: hidden;
    scrollbar-width: none;
    min-width: 0;
  }
  .filter-row::-webkit-scrollbar {
    display: none;
  }
  .filter-row.settings-spacer {
    padding: 0;
  }
  .filter-empty {
    color: var(--text-mute);
    font-size: 10.5px;
    font-style: italic;
    letter-spacing: 0.02em;
    padding: 6px 0;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px;
    padding: 4px 0;
    width: 100%;
  }
  .filter-chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 22px;
    padding: 0 8px;
    border: 0;
    border-radius: 11px;
    background: var(--card);
    color: var(--text-dim);
    font: inherit;
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
    white-space: nowrap;
    flex-shrink: 0;
    line-height: 1;
    letter-spacing: 0.005em;
    transition:
      background 0.15s ease,
      color 0.15s ease,
      transform 0.15s cubic-bezier(0.2, 0.8, 0.2, 1);
  }
  .filter-chip:hover {
    background: var(--card-hover);
    color: var(--text);
  }
  .filter-chip:active {
    transform: scale(0.96);
  }
  .filter-chip .chip-count {
    font-variant-numeric: tabular-nums;
    font-size: 9.5px;
    color: var(--text-mute);
    font-weight: 700;
  }
  .filter-chip .chip-dot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--text-mute);
    flex-shrink: 0;
  }
  .filter-chip.type-mode .chip-dot {
    background: var(--accent-blue-dim);
  }
  .filter-chip.type-mode.mode-acceptEdits .chip-dot {
    background: var(--accent-green-dim);
  }
  .filter-chip.type-mode.mode-bypassPermissions .chip-dot {
    background: var(--accent-red-dim);
  }

  /* selected (on) state */
  .filter-chip.on {
    background: color-mix(in oklch, var(--accent-green) 22%, transparent);
    color: var(--accent-green);
  }
  .filter-chip.on:hover {
    background: color-mix(in oklch, var(--accent-green) 28%, transparent);
  }
  .filter-chip.on .chip-count {
    color: var(--accent-green);
    opacity: 0.7;
  }
  .filter-chip.type-mode.mode-plan.on {
    background: color-mix(in oklch, var(--accent-blue) 22%, transparent);
    color: var(--accent-blue);
  }
  .filter-chip.type-mode.mode-plan.on .chip-dot {
    background: var(--accent-blue);
  }
  .filter-chip.type-mode.mode-plan.on .chip-count {
    color: var(--accent-blue);
  }
  .filter-chip.type-mode.mode-acceptEdits.on .chip-dot {
    background: var(--accent-green);
  }
  .filter-chip.type-mode.mode-bypassPermissions.on {
    background: color-mix(in oklch, var(--accent-red) 22%, transparent);
    color: var(--accent-red);
  }
  .filter-chip.type-mode.mode-bypassPermissions.on .chip-dot {
    background: var(--accent-red);
  }
  .filter-chip.type-mode.mode-bypassPermissions.on .chip-count {
    color: var(--accent-red);
  }
  .filter-chip.on .chip-dot {
    background: currentColor;
  }
  .filter-chip.clear-chip {
    background: transparent;
    color: var(--text-faint);
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: lowercase;
  }
  .filter-chip.clear-chip:hover {
    color: var(--accent-red);
    background: color-mix(in oklch, var(--accent-red) 12%, transparent);
  }

  /* ── settings panel ───────────────────────── */
  .settings-panel {
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 10px 14px 14px;
    animation: settings-in 220ms cubic-bezier(0.2, 0.8, 0.2, 1);
  }
  @keyframes settings-in {
    from {
      opacity: 0;
      transform: translateY(-6px);
    }
    to {
      opacity: 1;
      transform: none;
    }
  }
  .settings-row {
    display: grid;
    grid-template-columns: 60px 1fr;
    align-items: center;
    gap: 10px;
  }
  .settings-label {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.1em;
    color: var(--text-faint);
    font-weight: 700;
  }
  .seg {
    display: flex;
    align-items: stretch;
    background: var(--card);
    border-radius: 8px;
    padding: 2px;
    gap: 2px;
  }
  .seg button {
    flex: 1;
    background: transparent;
    border: none;
    font: inherit;
    color: var(--text-dim);
    font-size: 11px;
    font-weight: 600;
    padding: 5px 4px;
    border-radius: 6px;
    cursor: pointer;
    font-variant-numeric: tabular-nums;
    transition: background 0.15s ease, color 0.15s ease, transform 0.12s ease;
  }
  .seg button:hover {
    color: var(--text);
  }
  .seg button:active {
    transform: scale(0.96);
  }
  .seg button.active {
    background: var(--card-hover);
    color: var(--accent-green);
    box-shadow: inset 0 0 0 0.5px color-mix(in oklch, var(--accent-green) 35%, transparent);
  }
  .path-input {
    background: var(--card);
    border: none;
    border-radius: 8px;
    padding: 6px 9px;
    font: inherit;
    font-family: 'SF Mono', Menlo, monospace;
    font-size: 11px;
    color: var(--text);
    width: 100%;
    box-shadow: inset 0 0 0 0.5px var(--border-soft);
    transition: box-shadow 0.15s ease, background 0.15s ease;
  }
  .path-input:hover {
    background: var(--card-hover);
  }
  .path-input:focus {
    outline: none;
    background: var(--card-hover);
    box-shadow: inset 0 0 0 1px color-mix(in oklch, var(--accent-green) 45%, transparent);
  }
  .path-input::placeholder {
    color: var(--text-mute);
  }
  .settings-hint {
    margin: 4px 2px 0;
    color: var(--text-faint);
    font-size: 10.5px;
    line-height: 1.4;
  }
  .settings-hint code {
    background: var(--card);
    padding: 1px 5px;
    border-radius: 4px;
    color: var(--text-dim);
    font-family: 'SF Mono', Menlo, monospace;
    font-size: 10px;
  }
  .settings-actions {
    display: flex;
    justify-content: flex-end;
    padding-top: 4px;
    border-top: 0.5px solid var(--border-soft);
    margin-top: 4px;
  }
  .quit-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    background: transparent;
    border: none;
    color: var(--text-faint);
    font: inherit;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.01em;
    padding: 6px 10px;
    border-radius: 7px;
    cursor: pointer;
    transition: background 0.15s ease, color 0.15s ease, transform 0.12s ease;
  }
  .quit-btn:hover {
    background: color-mix(in oklch, var(--accent-red) 14%, transparent);
    color: var(--accent-red);
  }
  .quit-btn:active {
    transform: scale(0.97);
  }

  /* ── list ─────────────────────────────────── */
  .list {
    overflow-y: auto;
    overflow-x: hidden;
    padding: 4px 10px 10px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
  }
  .list.no-scroll {
    overflow: hidden;
  }

  .card {
    position: relative;
    padding: 12px 14px;
    background: var(--card);
    border-radius: 14px;
    display: grid;
    grid-template-rows: auto auto;
    gap: 11px;
    min-height: 58px;
    opacity: 0;
    transform: translateY(4px);
    animation: stagger 320ms cubic-bezier(0.2, 0.8, 0.2, 1) forwards;
  }
  @keyframes stagger {
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }
  .card:hover {
    background: var(--card-hover);
  }

  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 16px;
  }
  .row.top {
    color: var(--text-dim);
    font-size: 12px;
    font-weight: 500;
    justify-content: space-between;
    gap: 10px;
  }
  .row.bar {
    flex-direction: column;
    align-items: stretch;
    min-height: 8px;
    justify-content: center;
  }

  /* ── status dot ───────────────────────────── */
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
    box-shadow: 0 0 0 0 transparent;
  }
  .dot.status-running {
    background: var(--accent-yellow);
    animation: ring 2.2s ease-in-out infinite;
  }
  .dot.status-finished {
    background: var(--accent-green);
  }
  .dot.status-stopped {
    background: var(--text-mute);
  }
  @keyframes ring {
    0%,
    100% {
      box-shadow: 0 0 0 0 color-mix(in oklch, var(--accent-yellow) 50%, transparent);
    }
    50% {
      box-shadow: 0 0 0 5px color-mix(in oklch, var(--accent-yellow) 0%, transparent);
    }
  }

  /* ── cwd as card title ────────────────────── */
  .identity {
    display: flex;
    align-items: center;
    gap: 7px;
    flex: 1;
    min-width: 0;
  }
  .cwd {
    flex: 0 1 auto;
    min-width: 0;
    max-width: 104px;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    color: var(--text);
    font-weight: 700;
    font-size: 13px;
    letter-spacing: -0.01em;
  }
  .rel {
    flex-shrink: 0;
    font-variant-numeric: tabular-nums;
    font-size: 11px;
    color: var(--text-faint);
    font-weight: 500;
  }

  /* ── segment timeline ─────────────────────── */
  .bar-track {
    position: relative;
    width: 100%;
    height: 8px;
    background: var(--overlay-soft);
    border-radius: 4px;
    overflow: visible;
  }
  .bar-track::before {
    content: '';
    position: absolute;
    inset: 0;
    border-radius: inherit;
    overflow: hidden;
  }
  .bar-seg {
    position: absolute;
    top: 0;
    bottom: 0;
    padding: 0;
    border: 0;
    background: var(--overlay-medium);
    border-radius: 4px;
    cursor: default;
    outline: none;
    transition:
      background 0.18s ease,
      transform 0.18s cubic-bezier(0.16, 1, 0.3, 1);
  }
  .bar-seg:hover,
  .bar-seg:focus-visible {
    background: var(--text-dim);
    transform: scaleY(1.45);
    z-index: 3;
  }
  .bar-seg::after {
    content: attr(data-duration);
    position: absolute;
    left: 50%;
    bottom: calc(100% + 7px);
    transform: translateX(-50%) translateY(3px);
    padding: 3px 6px;
    border-radius: 6px;
    background: var(--tooltip-bg);
    box-shadow:
      inset 0 0 0 0.5px var(--tooltip-border),
      0 8px 18px rgb(0 0 0 / 0.35);
    color: var(--tooltip-text);
    font-size: 10px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    line-height: 1.2;
    opacity: 0;
    pointer-events: none;
    white-space: nowrap;
    z-index: 4;
    transition:
      opacity 0.14s ease,
      transform 0.14s cubic-bezier(0.16, 1, 0.3, 1);
  }
  .bar-seg:hover::after,
  .bar-seg:focus-visible::after {
    opacity: 1;
    transform: translateX(-50%) translateY(0);
  }
  .bar-seg.is-display {
    background: var(--text-mute);
  }
  .status-running .bar-seg.is-display {
    background: var(--accent-yellow);
    box-shadow: 0 0 8px color-mix(in oklch, var(--accent-yellow) 55%, transparent);
    animation: barPulse 2.4s ease-in-out infinite;
  }
  .status-finished .bar-seg.is-display {
    background: var(--accent-green);
    box-shadow: 0 0 0 0.5px color-mix(in oklch, var(--accent-green) 40%, transparent);
  }
  .status-stopped .bar-seg.is-display {
    background: var(--neutral-mid);
    box-shadow: 0 0 0 0.5px var(--overlay);
  }
  .bar-live {
    position: absolute;
    top: -2px;
    bottom: -2px;
    right: 0;
    width: 2px;
    background: var(--accent-yellow);
    border-radius: 1px;
    box-shadow: 0 0 8px color-mix(in oklch, var(--accent-yellow) 85%, transparent);
    animation: barCursor 1.1s ease-in-out infinite;
  }
  @keyframes barPulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.75;
    }
  }
  @keyframes barCursor {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.4;
    }
  }

  /* ── inline meta ──────────────────────────── */
  .meta {
    display: flex;
    align-items: center;
    gap: 5px;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    font-size: 10.5px;
    color: var(--text-dim);
  }
  .chip {
    font-size: 9.5px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    padding: 2px 6px;
    border-radius: 6px;
    background: var(--overlay);
    color: var(--text-dim);
    line-height: 1.5;
  }
  .chip.source-codex {
    background: color-mix(in oklch, var(--accent-blue) 22%, transparent);
    color: var(--accent-blue);
  }
  .chip.source-claude {
    background: color-mix(in oklch, var(--accent-yellow) 22%, transparent);
    color: var(--accent-yellow);
  }
  .chip.mode-plan {
    background: color-mix(in oklch, var(--accent-blue) 22%, transparent);
    color: var(--accent-blue);
  }
  .chip.goal-active {
    background: color-mix(in oklch, var(--accent-green) 22%, transparent);
    color: var(--accent-green);
  }
  .chip.goal-complete {
    background: color-mix(in oklch, var(--accent-yellow) 20%, transparent);
    color: var(--accent-yellow);
  }
  .chip.goal-unknown {
    background: var(--overlay);
    color: var(--text-dim);
  }
  .chip.mode-bypassPermissions {
    background: color-mix(in oklch, var(--accent-red) 22%, transparent);
    color: var(--accent-red);
  }
  .chip.mode-acceptEdits {
    background: color-mix(in oklch, var(--accent-green) 22%, transparent);
    color: var(--accent-green);
  }
  .meta-item {
    font-variant-numeric: tabular-nums;
    font-size: 10.5px;
    color: var(--text-dim);
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .meta-item.dim,
  .dim {
    color: var(--text-faint);
  }

  /* ── skeletons & empty ────────────────────── */
  .skeleton {
    height: 58px;
    border-radius: 14px;
    background: linear-gradient(
      90deg,
      var(--card) 0%,
      var(--card-hover) 50%,
      var(--card) 100%
    );
    background-size: 200% 100%;
    animation: shimmer 1.6s ease-in-out infinite;
  }
  @keyframes shimmer {
    0% {
      background-position: 200% 0;
    }
    100% {
      background-position: -200% 0;
    }
  }
  .empty {
    margin: auto;
    display: flex;
    flex-direction: column;
    gap: 8px;
    text-align: center;
    padding: 32px 24px;
    align-items: center;
  }
  .empty-line {
    color: var(--text-dim);
    font-size: 12px;
  }
  .empty-line code {
    background: var(--card);
    padding: 2px 6px;
    border-radius: 4px;
    color: var(--text);
    font-family: 'SF Mono', Menlo, monospace;
    font-size: 11px;
  }
  .empty-action {
    margin-top: 4px;
    background: var(--card);
    color: var(--text-dim);
    border: none;
    border-radius: 8px;
    padding: 6px 12px;
    font: inherit;
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
    transition: background 0.15s, color 0.15s;
  }
  .empty-action:hover {
    background: var(--card-hover);
    color: var(--text);
  }

  /* ── status bar ───────────────────────────── */
  .statusbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 16px 0 12px;
    background: var(--bg);
    color: var(--text-faint);
    font-size: 10.5px;
    font-variant-numeric: tabular-nums;
    font-weight: 500;
    min-width: 0;
  }
  .statusbar > span {
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .statusbar-actions {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    flex-shrink: 0;
    margin-left: 8px;
  }
  .count {
    color: var(--text);
    font-weight: 700;
  }
  .sep {
    margin: 0 6px;
    color: var(--text-mute);
  }
  .running {
    color: var(--accent-yellow);
    font-weight: 600;
  }
  .filtered {
    color: var(--accent-green);
    font-weight: 600;
  }
</style>
