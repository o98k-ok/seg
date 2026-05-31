<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { onMount } from 'svelte';
  import { toPng, toBlob } from 'html-to-image';
  import { save } from '@tauri-apps/plugin-dialog';
  import { writeImage } from '@tauri-apps/plugin-clipboard-manager';
  import { Image } from '@tauri-apps/api/image';
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

  // Share-card modal
  let shareSession = $state<Session | null>(null);
  let shareBusy = $state(false);
  let toast = $state<string | null>(null);
  let shareCardEl: HTMLDivElement | null = $state(null);
  let toastTimer: ReturnType<typeof setTimeout> | undefined;

  function showToast(msg: string) {
    toast = msg;
    if (toastTimer) clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast = null), 2200);
  }

  function openShare(s: Session) {
    shareSession = s;
  }
  function closeShare() {
    if (shareBusy) return;
    shareSession = null;
  }
  function handleKey(e: KeyboardEvent) {
    if (e.key === 'Escape' && shareSession) closeShare();
  }

  async function downloadShare() {
    if (!shareCardEl || shareBusy) return;
    shareBusy = true;
    try {
      const dataUrl = await toPng(shareCardEl, { pixelRatio: 3, cacheBust: true });
      const name = shareSession ? projectName(shareSession.cwd).name : 'thread';
      // Keep the popover from auto-hiding while the native save dialog has focus.
      await invoke('set_hide_suppressed', { v: true });
      try {
        const path = await save({
          defaultPath: `seg-${name}.png`,
          filters: [{ name: 'PNG Image', extensions: ['png'] }],
        });
        if (path) {
          await invoke('save_png', { path, base64Data: dataUrl.split(',')[1] });
          showToast('Saved ✓');
        }
      } finally {
        await invoke('set_hide_suppressed', { v: false });
      }
    } catch (err) {
      console.error('save failed', err);
      showToast('Save failed');
    } finally {
      shareBusy = false;
    }
  }

  async function copyShare() {
    if (!shareCardEl || shareBusy) return;
    shareBusy = true;
    try {
      const blob = await toBlob(shareCardEl, { pixelRatio: 3, cacheBust: true });
      if (!blob) throw new Error('no image produced');
      const bytes = new Uint8Array(await blob.arrayBuffer());
      await writeImage(await Image.fromBytes(bytes));
      showToast('Copied ✓');
    } catch (err) {
      console.error('copy failed', err);
      showToast('Copy failed');
    } finally {
      shareBusy = false;
    }
  }

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

<svelte:window onkeydown={handleKey} />

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
        <!-- svelte-ignore a11y_no_noninteractive_element_to_interactive_role -->
        <article
          class="card status-{s.status}"
          style="animation-delay: {Math.min(i, 11) * 50}ms"
          role="button"
          tabindex="0"
          onclick={() => openShare(s)}
          onkeydown={(e) => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault();
              openShare(s);
            }
          }}
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
                  onclick={(e) => e.stopPropagation()}
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

  {#if shareSession}
  {@const ss = shareSession}
  {@const sp = projectName(ss.cwd)}
  {@const longest =
    ss.segments.reduce((m, sg) => Math.max(m, sg.duration_secs), 0) ||
    ss.display_segment.duration_secs}
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
  <div class="share-overlay" role="presentation" onclick={closeShare}>
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div
      class="share-dialog"
      role="dialog"
      aria-modal="true"
      aria-label="Share thread card"
      tabindex="-1"
      onclick={(e) => e.stopPropagation()}
    >
      <div class="share-capture" bind:this={shareCardEl}>
        <div class="share-card-inner status-{ss.status}">
          <div class="sc-head">
            <span class="sc-brand">seg</span>
            <span class="sc-badges">
              <span class="chip source-{ss.source}">{ss.source}</span>
              {#if ss.has_plan_mode}
                <span class="chip mode-plan">plan</span>
              {/if}
              {#if ss.goal}
                <span class="chip goal-{ss.goal.status ?? 'active'}">goal</span>
              {/if}
              <span class="sc-status sc-status-{ss.status}">{ss.status}</span>
            </span>
          </div>

          <div class="sc-title">
            <span class="sc-project" title={sp.full}>{sp.name}</span>
            {#if ss.git_branch}
              <span class="sc-branch">⎇ {ss.git_branch}</span>
            {/if}
          </div>

          <div class="sc-mid">
            {#if ss.goal}
              <div class="sc-goal">{ss.goal.objective}</div>
            {:else}
              <div class="sc-bar-track">
                {#each ss.segments as seg, segI (segI)}
                  {@const layout = barSegLayout(ss, seg)}
                  <span
                    class="sc-bar-seg"
                    class:is-display={layout.isDisplay}
                    style="left: {layout.left}%; width: {layout.width}%"
                  ></span>
                {/each}
              </div>
            {/if}
          </div>

          <div class="sc-hero">
            <span class="sc-hero-val">{formatDuration(longest)}</span>
            <span class="sc-hero-label">longest turn</span>
          </div>

          <div class="sc-statline">
            <span><b>{formatDuration(ss.total_duration_secs)}</b> total</span>
            <span><b>{ss.total_turns}</b> turns</span>
            <span><b>{ss.segment_count}</b> segs</span>
            {#if ss.model}<span class="sc-model">{ss.model}</span>{/if}
          </div>

          <div class="sc-foot">
            <span class="sc-dates"
              >{new Date(ss.started_at).toLocaleDateString()} → {new Date(
                ss.last_event_at,
              ).toLocaleDateString()}</span
            >
            <span class="sc-foot-tag">generated by seg</span>
          </div>
        </div>
      </div>

      <div class="share-actions">
        <button class="share-btn primary" onclick={downloadShare} disabled={shareBusy}>
          {shareBusy ? '…' : 'Download'}
        </button>
        <button class="share-btn" onclick={copyShare} disabled={shareBusy}>Copy</button>
        <button class="share-btn ghost" onclick={closeShare} disabled={shareBusy}>Close</button>
      </div>
    </div>
  </div>
{/if}

{#if toast}
    <div class="toast">{toast}</div>
  {/if}
</div>

<style>
  :global(*) {
    box-sizing: border-box;
  }
  :global(html), :global(body) {
    background: transparent;
    font-family: ui-monospace, 'SF Mono', 'SFMono-Regular', Menlo, Consolas, monospace;
  }

  .shell {
    --font-mono: ui-monospace, 'SF Mono', 'SFMono-Regular', Menlo, Consolas, monospace;
    position: fixed;
    inset: 0;
    background: var(--bg);
    border: 4px solid var(--border);
    border-radius: 0;
    display: grid;
    grid-template-rows: auto auto 1fr 34px;
    grid-template-columns: minmax(0, 1fr);
    overflow: hidden;
    color: var(--text);
    font-family: var(--font-mono);
    transition: background 0.1s linear, color 0.1s linear;
  }

  /* ── theme tokens (Neo-Brutalist) ─────────── */
  .shell[data-theme='dark'] {
    --bg: #0a0a0a;
    --card: #161616;
    --card-hover: #242424;
    --border: #ffffff;
    --border-soft: #ffffff;

    --text: #ffffff;
    --text-dim: #eaeaea;
    --text-faint: #aaaaaa;
    --text-mute: #777777;

    --on-accent: #000000;

    --accent-green: #ccff00;
    --accent-green-dim: #84a300;
    --accent-blue: #00d9ff;
    --accent-blue-dim: #0090aa;
    --accent-yellow: #ff9500;
    --accent-yellow-dim: #a86200;
    --accent-red: #ff006e;
    --accent-red-dim: #a8004a;

    --overlay-soft: #242424;
    --overlay: #2e2e2e;
    --overlay-medium: #8a8a8a;
    --overlay-border: #ffffff;
    --neutral-mid: #777777;

    --tooltip-bg: #000000;
    --tooltip-text: #ffffff;
    --tooltip-border: #ffffff;
  }

  .shell[data-theme='light'] {
    --bg: #ffffff;
    --card: #ffffff;
    --card-hover: #f0f0f0;
    --border: #000000;
    --border-soft: #000000;

    --text: #000000;
    --text-dim: #000000;
    --text-faint: #333333;
    --text-mute: #555555;

    --on-accent: #000000;

    --accent-green: #ccff00;
    --accent-green-dim: #aacc00;
    --accent-blue: #00d9ff;
    --accent-blue-dim: #00b0d0;
    --accent-yellow: #ff9500;
    --accent-yellow-dim: #d97a00;
    --accent-red: #ff006e;
    --accent-red-dim: #d0005a;

    --overlay-soft: #f0f0f0;
    --overlay: #e6e6e6;
    --overlay-medium: #555555;
    --overlay-border: #000000;
    --neutral-mid: #999999;

    --tooltip-bg: #000000;
    --tooltip-text: #ffffff;
    --tooltip-border: #000000;
  }

  /* ── top bar (tabs + gear) ───────────────── */
  .topbar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 5px 10px;
    min-height: 40px;
    background: var(--bg);
    border-bottom: 3px solid var(--border);
    min-width: 0;
  }
  .tabs {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px;
    flex: 1;
    min-width: 0;
    overflow: visible;
  }
  .tabs button {
    background: var(--card);
    border: 2px solid var(--border);
    color: var(--text);
    cursor: pointer;
    font: inherit;
    font-family: var(--font-mono);
    font-size: 10.5px;
    font-weight: 900;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 3px 7px;
    border-radius: 0;
    transition: transform 0.08s linear, background 0.08s linear, color 0.08s linear;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    flex-shrink: 0;
  }
  .tabs button:hover {
    background: var(--card-hover);
  }
  .tabs button.active {
    color: var(--bg);
    background: var(--border);
  }
  .tabs button.active .tab-count {
    color: var(--bg);
  }
  .tab-count {
    font-variant-numeric: tabular-nums;
    color: var(--text-mute);
    font-weight: 900;
    font-size: 10px;
  }
  .status-tab .status-dot {
    width: 7px;
    height: 7px;
    border-radius: 0;
    border: 1px solid var(--border);
    flex-shrink: 0;
  }
  .status-tab.status-running .status-dot {
    background: var(--accent-yellow);
  }
  .status-tab.status-running.active .status-dot {
    animation: blink 1s steps(1) infinite;
  }
  .status-tab.status-running.active {
    color: var(--on-accent);
    background: var(--accent-yellow);
  }
  .status-tab.status-running.active .tab-count {
    color: var(--on-accent);
  }
  .status-tab.status-finished .status-dot {
    background: var(--accent-green);
  }
  .status-tab.status-finished.active .status-dot {
    border-color: var(--border);
  }
  .status-tab.status-finished.active {
    color: var(--on-accent);
    background: var(--accent-green);
  }
  .status-tab.status-finished.active .tab-count {
    color: var(--on-accent);
  }
  .icon-btn {
    width: 22px;
    height: 22px;
    background: var(--card);
    border: 2px solid var(--border);
    border-radius: 0;
    color: var(--text);
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    padding: 0;
    box-shadow: 2px 2px 0 0 var(--border);
    transition: transform 0.08s linear, background 0.08s linear, color 0.08s linear, box-shadow 0.08s linear;
  }
  .icon-btn:hover:not(:disabled) {
    background: var(--card-hover);
    transform: translate(-1px, -1px);
    box-shadow: 3px 3px 0 0 var(--border);
  }
  .icon-btn:active:not(:disabled) {
    transform: translate(2px, 2px);
    box-shadow: 0 0 0 0 var(--border);
  }
  .icon-btn:disabled {
    cursor: default;
    opacity: 0.5;
  }
  .refresh {
    font-size: 14px;
    font-weight: 900;
    line-height: 1;
  }
  .refresh.loading {
    animation: spin 0.8s steps(8) infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .gear.active {
    background: var(--accent-blue);
    color: var(--on-accent);
  }
  .gear.active:active {
    transform: translate(2px, 2px);
    box-shadow: 0 0 0 0 var(--border);
  }

  /* ── filter row (replaces search) ────────── */
  .filter-row {
    display: flex;
    align-items: flex-start;
    padding: 0 10px;
    background: var(--bg);
    border-bottom: 3px solid var(--border);
    max-height: 64px;
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
    border-bottom: 0;
    max-height: none;
  }
  .filter-empty {
    color: var(--text-mute);
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    padding: 7px 0;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 5px;
    padding: 5px 0;
    width: 100%;
  }
  .filter-chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 22px;
    padding: 0 7px;
    border: 2px solid var(--border);
    border-radius: 0;
    background: var(--card);
    color: var(--text);
    font: inherit;
    font-family: var(--font-mono);
    font-size: 10.5px;
    font-weight: 800;
    cursor: pointer;
    white-space: nowrap;
    flex-shrink: 0;
    line-height: 1;
    letter-spacing: 0.01em;
    transition: transform 0.08s linear, background 0.08s linear, color 0.08s linear, box-shadow 0.08s linear;
  }
  .filter-chip:hover {
    background: var(--card-hover);
    transform: translate(-1px, -1px);
    box-shadow: 2px 2px 0 0 var(--border);
  }
  .filter-chip:active {
    transform: translate(1px, 1px);
    box-shadow: 0 0 0 0 var(--border);
  }
  .filter-chip .chip-count {
    font-variant-numeric: tabular-nums;
    font-size: 9.5px;
    color: var(--text-mute);
    font-weight: 900;
  }
  .filter-chip .chip-dot {
    width: 6px;
    height: 6px;
    border-radius: 0;
    background: var(--text-mute);
    border: 1px solid var(--border);
    flex-shrink: 0;
  }
  .filter-chip.type-mode .chip-dot {
    background: var(--accent-blue);
  }
  .filter-chip.type-mode.mode-acceptEdits .chip-dot {
    background: var(--accent-green);
  }
  .filter-chip.type-mode.mode-bypassPermissions .chip-dot {
    background: var(--accent-red);
  }

  /* selected (on) state — solid neon block, black text */
  .filter-chip.on {
    background: var(--accent-green);
    color: var(--on-accent);
  }
  .filter-chip.on:hover {
    background: var(--accent-green);
  }
  .filter-chip.on .chip-count {
    color: var(--on-accent);
  }
  .filter-chip.type-mode.mode-plan.on {
    background: var(--accent-blue);
    color: var(--on-accent);
  }
  .filter-chip.type-mode.mode-plan.on .chip-dot {
    background: var(--on-accent);
  }
  .filter-chip.type-mode.mode-plan.on .chip-count {
    color: var(--on-accent);
  }
  .filter-chip.type-mode.mode-acceptEdits.on .chip-dot {
    background: var(--on-accent);
  }
  .filter-chip.type-mode.mode-bypassPermissions.on {
    background: var(--accent-red);
    color: var(--on-accent);
  }
  .filter-chip.type-mode.mode-bypassPermissions.on .chip-dot {
    background: var(--on-accent);
  }
  .filter-chip.type-mode.mode-bypassPermissions.on .chip-count {
    color: var(--on-accent);
  }
  .filter-chip.on .chip-dot {
    background: var(--on-accent);
  }
  .filter-chip.clear-chip {
    background: var(--card);
    color: var(--accent-red);
    font-weight: 900;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .filter-chip.clear-chip:hover {
    color: var(--on-accent);
    background: var(--accent-red);
  }

  /* ── settings panel ───────────────────────── */
  .settings-panel {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 12px 14px 14px;
  }
  .settings-row {
    display: grid;
    grid-template-columns: 56px 1fr;
    align-items: center;
    gap: 10px;
  }
  .settings-label {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.1em;
    color: var(--text);
    font-weight: 900;
  }
  .seg {
    display: flex;
    align-items: stretch;
    background: var(--card);
    border: 2px solid var(--border);
    border-radius: 0;
    padding: 0;
    gap: 0;
  }
  .seg button {
    flex: 1;
    background: var(--card);
    border: 0;
    border-right: 2px solid var(--border);
    font: inherit;
    font-family: var(--font-mono);
    color: var(--text);
    font-size: 10.5px;
    font-weight: 800;
    padding: 5px 4px;
    border-radius: 0;
    cursor: pointer;
    font-variant-numeric: tabular-nums;
    transition: background 0.08s linear, color 0.08s linear;
  }
  .seg button:last-child {
    border-right: 0;
  }
  .seg button:hover {
    background: var(--card-hover);
  }
  .seg button.active {
    background: var(--accent-green);
    color: var(--on-accent);
  }
  .path-input {
    background: var(--card);
    border: 2px solid var(--border);
    border-radius: 0;
    padding: 6px 9px;
    font: inherit;
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--text);
    width: 100%;
    transition: box-shadow 0.08s linear, background 0.08s linear;
  }
  .path-input:hover {
    background: var(--card-hover);
  }
  .path-input:focus {
    outline: none;
    background: var(--card);
    box-shadow: 3px 3px 0 0 var(--accent-blue);
  }
  .path-input::placeholder {
    color: var(--text-mute);
  }
  .settings-hint {
    margin: 2px 0 0;
    color: var(--text-faint);
    font-size: 10px;
    line-height: 1.5;
  }
  .settings-hint code {
    background: var(--accent-blue);
    padding: 1px 5px;
    border-radius: 0;
    color: var(--on-accent);
    font-family: var(--font-mono);
    font-size: 10px;
    font-weight: 700;
  }
  .settings-actions {
    display: flex;
    justify-content: flex-end;
    padding-top: 10px;
    border-top: 3px solid var(--border);
    margin-top: 2px;
  }
  .quit-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    background: var(--card);
    border: 2px solid var(--border);
    color: var(--text);
    font: inherit;
    font-family: var(--font-mono);
    font-size: 10.5px;
    font-weight: 900;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 6px 10px;
    border-radius: 0;
    cursor: pointer;
    box-shadow: 2px 2px 0 0 var(--border);
    transition: background 0.08s linear, color 0.08s linear, transform 0.08s linear, box-shadow 0.08s linear;
  }
  .quit-btn:hover {
    background: var(--accent-red);
    color: var(--on-accent);
    transform: translate(-1px, -1px);
    box-shadow: 3px 3px 0 0 var(--border);
  }
  .quit-btn:active {
    transform: translate(2px, 2px);
    box-shadow: 0 0 0 0 var(--border);
  }

  /* ── list ─────────────────────────────────── */
  .list {
    overflow-y: auto;
    overflow-x: hidden;
    padding: 8px 14px 14px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 0;
  }
  .list.no-scroll {
    overflow: hidden;
  }

  .card {
    position: relative;
    padding: 11px 13px;
    background: var(--card);
    border: 3px solid var(--border);
    border-radius: 0;
    box-shadow: 4px 4px 0 0 var(--border);
    display: grid;
    grid-template-rows: auto auto;
    gap: 10px;
    min-height: 56px;
    transition: transform 0.08s linear, box-shadow 0.08s linear, background 0.08s linear;
  }
  .card:hover {
    background: var(--card-hover);
    transform: translate(-2px, -2px);
    box-shadow: 6px 6px 0 0 var(--border);
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
    font-weight: 700;
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
    width: 10px;
    height: 10px;
    border-radius: 0;
    border: 2px solid var(--border);
    flex-shrink: 0;
  }
  .dot.status-running {
    background: var(--accent-yellow);
    animation: blink 1s steps(1) infinite;
  }
  .dot.status-finished {
    background: var(--accent-green);
  }
  .dot.status-stopped {
    background: var(--neutral-mid);
  }
  @keyframes blink {
    50% {
      opacity: 0.25;
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
    font-weight: 900;
    font-size: 13px;
    letter-spacing: -0.01em;
  }
  .rel {
    flex-shrink: 0;
    font-variant-numeric: tabular-nums;
    font-size: 10.5px;
    color: var(--text-faint);
    font-weight: 700;
  }

  /* ── segment timeline ─────────────────────── */
  .bar-track {
    position: relative;
    width: 100%;
    height: 10px;
    background: var(--overlay-soft);
    border: 2px solid var(--border);
    border-radius: 0;
    overflow: visible;
  }
  .bar-track::before {
    content: '';
    position: absolute;
    inset: 0;
    overflow: hidden;
  }
  .bar-seg {
    position: absolute;
    top: 0;
    bottom: 0;
    padding: 0;
    border: 0;
    background: var(--overlay-medium);
    border-radius: 0;
    cursor: default;
    outline: none;
    transition: background 0.08s linear, transform 0.08s linear;
  }
  .bar-seg:hover,
  .bar-seg:focus-visible {
    background: var(--text);
    transform: scaleY(1.4);
    z-index: 3;
  }
  .bar-seg::after {
    content: attr(data-duration);
    position: absolute;
    left: 50%;
    bottom: calc(100% + 7px);
    transform: translateX(-50%);
    padding: 3px 6px;
    border-radius: 0;
    background: var(--tooltip-bg);
    border: 2px solid var(--tooltip-border);
    box-shadow: 3px 3px 0 0 var(--tooltip-border);
    color: var(--tooltip-text);
    font-size: 10px;
    font-weight: 800;
    font-variant-numeric: tabular-nums;
    line-height: 1.2;
    opacity: 0;
    pointer-events: none;
    white-space: nowrap;
    z-index: 4;
    transition: opacity 0.08s linear;
  }
  .bar-seg:hover::after,
  .bar-seg:focus-visible::after {
    opacity: 1;
  }
  .bar-seg.is-display {
    background: var(--text-mute);
  }
  .status-running .bar-seg.is-display {
    background: var(--accent-yellow);
    animation: blink 1s steps(1) infinite;
  }
  .status-finished .bar-seg.is-display {
    background: var(--accent-green);
  }
  .status-stopped .bar-seg.is-display {
    background: var(--neutral-mid);
  }
  .bar-live {
    position: absolute;
    top: -2px;
    bottom: -2px;
    right: 0;
    width: 3px;
    background: var(--accent-yellow);
    border-radius: 0;
    animation: blink 0.8s steps(1) infinite;
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
    font-size: 9px;
    font-weight: 900;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    padding: 2px 5px;
    border-radius: 0;
    border: 1.5px solid var(--border);
    background: var(--card);
    color: var(--text);
    line-height: 1.5;
  }
  .chip.source-codex {
    background: var(--accent-blue);
    color: var(--on-accent);
  }
  .chip.source-claude {
    background: var(--accent-yellow);
    color: var(--on-accent);
  }
  .chip.mode-plan {
    background: var(--accent-blue);
    color: var(--on-accent);
  }
  .chip.goal-active {
    background: var(--accent-green);
    color: var(--on-accent);
  }
  .chip.goal-complete {
    background: var(--accent-yellow);
    color: var(--on-accent);
  }
  .chip.goal-unknown {
    background: var(--card);
    color: var(--text);
  }
  .chip.mode-bypassPermissions {
    background: var(--accent-red);
    color: var(--on-accent);
  }
  .chip.mode-acceptEdits {
    background: var(--accent-green);
    color: var(--on-accent);
  }
  .meta-item {
    font-variant-numeric: tabular-nums;
    font-size: 10px;
    color: var(--text-dim);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .meta-item.dim,
  .dim {
    color: var(--text-faint);
  }

  /* ── skeletons & empty ────────────────────── */
  .skeleton {
    height: 56px;
    border: 3px solid var(--border);
    border-radius: 0;
    background: var(--card-hover);
    box-shadow: 4px 4px 0 0 var(--border);
    animation: blink 1s steps(1) infinite;
  }
  .empty {
    margin: auto;
    display: flex;
    flex-direction: column;
    gap: 10px;
    text-align: center;
    padding: 28px 24px;
    align-items: center;
  }
  .empty-line {
    color: var(--text);
    font-size: 12px;
    font-weight: 700;
  }
  .empty-line code {
    background: var(--accent-blue);
    padding: 2px 6px;
    border-radius: 0;
    color: var(--on-accent);
    font-family: var(--font-mono);
    font-size: 11px;
    font-weight: 700;
  }
  .empty-action {
    margin-top: 4px;
    background: var(--card);
    color: var(--text);
    border: 2px solid var(--border);
    border-radius: 0;
    padding: 6px 12px;
    font: inherit;
    font-family: var(--font-mono);
    font-size: 10.5px;
    font-weight: 900;
    text-transform: uppercase;
    cursor: pointer;
    box-shadow: 2px 2px 0 0 var(--border);
    transition: background 0.08s linear, color 0.08s linear, transform 0.08s linear, box-shadow 0.08s linear;
  }
  .empty-action:hover {
    background: var(--accent-green);
    color: var(--on-accent);
    transform: translate(-1px, -1px);
    box-shadow: 3px 3px 0 0 var(--border);
  }
  .empty-action:active {
    transform: translate(2px, 2px);
    box-shadow: 0 0 0 0 var(--border);
  }

  /* ── status bar ───────────────────────────── */
  .statusbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 14px 0 12px;
    background: var(--bg);
    border-top: 3px solid var(--border);
    color: var(--text-faint);
    font-size: 10px;
    font-variant-numeric: tabular-nums;
    font-weight: 700;
    min-width: 0;
  }
  .statusbar > span {
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .statusbar-actions {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
    margin-left: 8px;
  }
  .count {
    color: var(--text);
    font-weight: 900;
  }
  .sep {
    margin: 0 5px;
    color: var(--text-mute);
  }
  .running {
    color: var(--text);
    font-weight: 900;
  }
  .filtered {
    color: var(--text);
    font-weight: 900;
  }

  /* ── share-card modal ─────────────────────── */
  .card {
    cursor: pointer;
  }
  .card:focus-visible {
    outline: 3px solid var(--accent-blue);
    outline-offset: -1px;
  }

  .share-overlay {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 12px;
    background: rgba(0, 0, 0, 0.6);
  }
  .share-dialog {
    display: flex;
    flex-direction: column;
    gap: 10px;
    align-items: stretch;
  }

  /* the exported node (square) */
  .share-capture {
    width: 320px;
    height: 320px;
    background: var(--bg);
    border: 4px solid var(--border);
    padding: 13px;
  }
  .share-card-inner {
    height: 100%;
    background: var(--card);
    border: 3px solid var(--border);
    box-shadow: 5px 5px 0 0 var(--border);
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 9px;
    overflow: hidden;
  }
  .sc-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 8px;
  }
  .sc-brand {
    background: var(--border);
    color: var(--bg);
    font-weight: 900;
    font-size: 15px;
    letter-spacing: 0.02em;
    padding: 2px 8px;
  }
  .sc-badges {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: flex-end;
    gap: 4px;
  }
  .sc-status {
    font-size: 9px;
    font-weight: 900;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    padding: 2px 6px;
    border: 1.5px solid var(--border);
    color: var(--on-accent);
    line-height: 1.5;
  }
  .sc-status-running {
    background: var(--accent-yellow);
  }
  .sc-status-finished {
    background: var(--accent-green);
  }
  .sc-status-stopped {
    background: var(--neutral-mid);
  }

  .sc-title {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .sc-project {
    font-size: 22px;
    font-weight: 900;
    letter-spacing: -0.02em;
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    line-height: 1.05;
  }
  .sc-branch {
    font-size: 11px;
    font-weight: 700;
    color: var(--text-faint);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .sc-mid {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    justify-content: center;
  }
  .sc-goal {
    border-left: 4px solid var(--accent-blue);
    background: var(--card-hover);
    padding: 7px 9px;
    font-size: 12px;
    font-weight: 600;
    line-height: 1.35;
    color: var(--text);
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .sc-bar-track {
    position: relative;
    width: 100%;
    height: 14px;
    background: var(--overlay-soft);
    border: 2px solid var(--border);
  }
  .sc-bar-seg {
    position: absolute;
    top: 0;
    bottom: 0;
    background: var(--overlay-medium);
  }
  .sc-bar-seg.is-display {
    background: var(--text-mute);
  }
  .status-running .sc-bar-seg.is-display {
    background: var(--accent-yellow);
  }
  .status-finished .sc-bar-seg.is-display {
    background: var(--accent-green);
  }
  .status-stopped .sc-bar-seg.is-display {
    background: var(--neutral-mid);
  }

  .sc-hero {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 8px;
    background: var(--accent-yellow);
    color: var(--on-accent);
    border: 2px solid var(--border);
    padding: 6px 10px;
  }
  .sc-hero-val {
    font-size: 26px;
    font-weight: 900;
    line-height: 1;
    font-variant-numeric: tabular-nums;
    letter-spacing: -0.02em;
  }
  .sc-hero-label {
    font-size: 9px;
    font-weight: 900;
    text-transform: uppercase;
    letter-spacing: 0.12em;
    color: var(--on-accent);
    flex-shrink: 0;
  }
  .sc-statline {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 3px 10px;
    font-size: 10px;
    font-weight: 600;
    color: var(--text-faint);
  }
  .sc-statline b {
    font-weight: 900;
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  .sc-model {
    margin-left: auto;
    font-size: 9px;
    font-weight: 700;
    border: 1.5px solid var(--border);
    padding: 1px 5px;
    background: var(--card);
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 130px;
  }
  .sc-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    font-size: 8.5px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-faint);
  }
  .sc-foot-tag {
    background: var(--accent-green);
    color: var(--on-accent);
    padding: 1px 5px;
    white-space: nowrap;
  }
  .sc-dates {
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  /* action bar (not captured) */
  .share-actions {
    display: flex;
    gap: 8px;
  }
  .share-btn {
    flex: 1;
    background: var(--card);
    color: var(--text);
    border: 2px solid var(--border);
    box-shadow: 3px 3px 0 0 var(--border);
    font: inherit;
    font-family: var(--font-mono);
    font-size: 11px;
    font-weight: 900;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 8px 6px;
    cursor: pointer;
    transition: transform 0.08s linear, background 0.08s linear, color 0.08s linear, box-shadow 0.08s linear;
  }
  .share-btn:hover:not(:disabled) {
    transform: translate(-1px, -1px);
    box-shadow: 4px 4px 0 0 var(--border);
  }
  .share-btn:active:not(:disabled) {
    transform: translate(3px, 3px);
    box-shadow: 0 0 0 0 var(--border);
  }
  .share-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .share-btn.primary {
    background: var(--accent-green);
    color: var(--on-accent);
  }
  .share-btn.ghost {
    color: var(--text-faint);
  }

  /* toast */
  .toast {
    position: fixed;
    left: 50%;
    bottom: 44px;
    transform: translateX(-50%);
    z-index: 60;
    background: var(--border);
    color: var(--bg);
    border: 2px solid var(--border);
    font-size: 11px;
    font-weight: 900;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    padding: 6px 12px;
    box-shadow: 3px 3px 0 0 var(--accent-blue);
    white-space: nowrap;
  }

  @media (prefers-reduced-motion: reduce) {
    .dot.status-running,
    .status-tab.status-running.active .status-dot,
    .status-running .bar-seg.is-display,
    .bar-live,
    .skeleton,
    .refresh.loading {
      animation: none;
    }
  }
</style>
