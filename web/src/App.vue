<script setup lang="ts">
import { computed, onMounted, onUnmounted, reactive, ref } from 'vue'
import { Api, ApiError, assertSafeNumbers, explain, label, parseConfiguration, type Configuration, type Json, type Operation, type Package, type Plugin } from './api'

type Page = 'overview' | 'plugins' | 'operations' | 'diagnostics'
type Editor = { data: Configuration; schema: Json; text: string; original: string; notice: string; error: string; pending?: string }
type Confirmation = { title: string; description: string; method: string; payload: Record<string, unknown>; grants?: string[]; required?: string[]; chosen?: string[]; self?: boolean }
type LogEntry = { operation_id: string; method: string; state: string; stage: string; timestamp_unix_ms: number; result: Json }
const api = new Api()
const authenticated = ref(false), booting = ref(true), busy = ref(false), refreshing = ref(false)
const writeAllowed = ref(false)
const token = ref(''), error = ref(''), notice = ref(''), page = ref<Page>('overview')
const plugins = ref<Plugin[]>([]), packages = ref<Package[]>([]), operations = ref<Operation[]>([])
const selected = ref(''), selectedVersion = ref(''), search = ref(''), listMode = ref<'running' | 'installed'>('running')
const installPath = ref(''), showInstall = ref(false), confirmation = ref<Confirmation | null>(null)
const editors = reactive<Record<string, Editor>>({}), configBusy = ref(false)
const logs = ref<LogEntry[]>([]), diagnosticError = ref(''), adapterError = ref(''), adapter = ref<Json | undefined>(), adapterId = ref('')
let timer: ReturnType<typeof setInterval> | undefined
let diagnosticBusy = false
let installedAt = 0
const current = computed(() => plugins.value.find(p => p.id === selected.value))
const versions = computed(() => packages.value.filter(p => p.id === selected.value))
const editor = computed(() => editors[selected.value])
const dirty = computed(() => editor.value && editor.value.text !== editor.value.original)
const activeCount = computed(() => plugins.value.filter(p => p.state === 'active').length)
const failedCount = computed(() => plugins.value.filter(p => p.state === 'failed' || p.state === 'degraded' || p.error).length)
const runningOps = computed(() => operations.value.filter(o => o.state === 'running'))
const visiblePlugins = computed(() => plugins.value.filter(p => `${p.id} ${p.version}`.toLowerCase().includes(search.value.toLowerCase())))
const visiblePackages = computed(() => packages.value.filter(p => `${p.id} ${p.version}`.toLowerCase().includes(search.value.toLowerCase())))
const recentOps = computed(() => [...operations.value].reverse())
const pretty = (value: unknown) => JSON.stringify(value, null, 2)
const time = (value?: number) => value ? new Date(value).toLocaleString('zh-CN', { hour12: false }) : '—'
function resetSession(message = '') {
  installedAt = 0
  authenticated.value = false; token.value = ''; api.invalidate(); writeAllowed.value = false; error.value = message
  plugins.value = []; packages.value = []; operations.value = []; logs.value = []; adapter.value = undefined
  diagnosticError.value = ''; adapterError.value = ''; adapterId.value = ''
  selected.value = ''; confirmation.value = null; notice.value = ''; installPath.value = ''; showInstall.value = false
  for (const key of Object.keys(editors)) delete editors[key]
}
api.onExpired = () => resetSession('会话已过期，请重新登录。')
async function refresh(forceInventory = false) {
  if (!authenticated.value || refreshing.value) return
  refreshing.value = true
  try {
    const [p, installed, ops, overview] = await Promise.all([api.rpc<Plugin[]>('plugins.list'), forceInventory || Date.now() - installedAt >= 30_000 ? api.rpc<Package[]>('plugins.installed') : Promise.resolve(packages.value), api.rpc<Operation[]>('operations.list'), api.rpc<{ permissions: { read: boolean; write: boolean } }>('management.overview')])
    if (!authenticated.value) return
    writeAllowed.value = overview.permissions?.write === true
    plugins.value = p; if (installed !== packages.value) installedAt = Date.now(); packages.value = installed; operations.value = ops
    for (const [id, draft] of Object.entries(editors)) {
      if (!draft.pending) continue
      const operation = ops.find(op => op.id === draft.pending)
      if (!operation || operation.state === 'running') continue
      draft.pending = undefined
      if (operation.state !== 'completed') {
        draft.error = /conflict/i.test(operation.result?.code ?? '')
          ? '配置修订冲突。编辑内容和基准修订已保留，请读取最新配置后核对。'
          : `配置更新${label(operation.state)}（${operation.result?.code ?? operation.state}）。输入已保留，请查看操作记录并核对期望与已应用修订。`
        draft.notice = ''
      } else draft.notice = '配置操作已完成。编辑内容仍保留原基准修订，请重新读取最新配置后继续编辑。'
      try {
        const latest = await api.rpc<Configuration>('config.get', { id })
        assertSafeNumbers(latest.status)
        if (authenticated.value) draft.data.status = latest.status
      } catch (e) { if (e instanceof ApiError && e.code === 'StaleSession') return; if (authenticated.value) draft.error = explain(e) }
    }
  } catch (e) { if (e instanceof ApiError && e.code === 'StaleSession') return; error.value = explain(e) } finally { refreshing.value = false }
}
async function login() {
  if (busy.value) return
  busy.value = true; error.value = ''
  const credential = token.value; token.value = ''
  try { await api.session(credential); authenticated.value = true; await refresh() }
  catch (e) { if (e instanceof ApiError && e.code === 'StaleSession') return; error.value = explain(e) }
  finally { busy.value = false }
}
async function logout() {
  if (busy.value) return
  busy.value = true
  try { await api.logout(); resetSession() }
  catch (e) { if (e instanceof ApiError && e.code === 'StaleSession') return; error.value = explain(e) }
  finally { busy.value = false }
}
async function choose(id: string, version: string) {
  selected.value = id; selectedVersion.value = version; page.value = 'plugins'
  if (plugins.value.some(p => p.id === id) && !editors[id]) await loadConfig(id)
}
async function loadConfig(id = selected.value, replace = false) {
  if (configBusy.value) return
  configBusy.value = true
  try {
    const data = await api.rpc<Configuration & { schema: Json }>('config.schema', { id })
    if (!authenticated.value) return
    assertSafeNumbers(data.snapshot); assertSafeNumbers(data.status)
    const text = pretty(data.snapshot.values)
    const previous = editors[id]
    if (previous && !replace) return
    editors[id] = { data, schema: data.schema, text, original: text, notice: '', error: '' }
  } catch (e) { if (e instanceof ApiError && e.code === 'StaleSession') return; error.value = explain(e) } finally { configBusy.value = false }
}
function reloadConfig() {
  if (dirty.value) {
    confirmation.value = { title: '重新载入配置', description: '这将用最新配置替换当前编辑内容。请先复制需要保留的修改。', method: 'local.reloadConfig', payload: { id: selected.value } }
  } else void loadConfig(selected.value, true)
}
async function validateConfig() {
  const target = editor.value
  if (!target || configBusy.value) return
  configBusy.value = true; target.error = ''; target.notice = ''
  try {
    const values = parseConfiguration(target.text)
    await api.rpc('config.validate', { id: selected.value, values })
    target.notice = '配置校验通过，可以保存。'
  } catch (e) { if (e instanceof ApiError && e.code === 'StaleSession') return; target.error = explain(e) } finally { configBusy.value = false }
}
function saveConfig() {
  const target = editor.value
  if (!target || !writeAllowed.value) return
  target.error = ''
  try {
    const values = parseConfiguration(target.text)
    confirmation.value = { title: '保存并应用配置', description: `提交基于修订 ${target.data.snapshot.revision} 的修改。运行中的插件将尝试应用配置，失败时请核对期望与已应用修订。`, method: 'config.update', payload: { id: selected.value, expected_revision: target.data.snapshot.revision, values }, self: selected.value === 'web-admin' }
  } catch (e) { if (e instanceof ApiError && e.code === 'StaleSession') return; target.error = explain(e) }
}
function askAction(method: string, id: string, version?: string) {
  if (!writeAllowed.value) return
  const descriptions: Record<string, string> = {
    'plugins.start': '启动该插件，并连接其外部服务。', 'plugins.stop': '停止该插件，正在处理的工作可能中断。',
    'plugins.reload': '重新启动该插件。连接和在途工作可能中断；新版本失败时宿主将尝试恢复旧版本。',
    'plugins.disable': '停止插件并取消自动启用，保留配置与数据。请先处理依赖它的插件。',
    'plugins.uninstall': '删除该已安装版本。配置与业务数据保留；正在启用的版本需要先停用。',
  }
  confirmation.value = { title: `${label(method)} · ${id}`, description: descriptions[method] ?? '', method, payload: { id, ...(version ? { version } : {}) }, self: id === 'web-admin' && ['plugins.stop', 'plugins.disable', 'plugins.reload'].includes(method) }
}
async function askEnable(pkg: Package) {
  if (!writeAllowed.value || busy.value) return
  busy.value = true; error.value = ''
  try {
    const { manifest } = await api.rpc<{ manifest: { capabilities: string[]; required_capabilities: string[] } }>('plugins.inspect', { id: pkg.id, version: pkg.version })
    if (!authenticated.value) return
    confirmation.value = { title: `授权并启用 · ${pkg.id}`, description: '仅向受信任的插件授予确有需要的能力。必需能力已选中，启用后将立即启动。', method: 'plugins.enable', payload: { id: pkg.id, version: pkg.version }, grants: manifest.capabilities, required: manifest.required_capabilities, chosen: [...manifest.required_capabilities] }
  } catch (e) { if (e instanceof ApiError && e.code === 'StaleSession') return; error.value = explain(e) } finally { busy.value = false }
}
function askInstall() {
  if (!writeAllowed.value || !installPath.value.trim()) return
  confirmation.value = { title: '安装本地插件包', description: '请确认来源可信。安装成功后仍需单独授权并启用。', method: 'plugins.install', payload: { path: installPath.value.trim() } }
}
async function confirmAction() {
  const action = confirmation.value
  if (!action || busy.value) return
  if (action.method === 'local.reloadConfig') { confirmation.value = null; await loadConfig(String(action.payload.id), true); return }
  if (!writeAllowed.value) { confirmation.value = null; error.value = '当前为只读模式，无法提交变更。'; return }
  busy.value = true; error.value = ''; notice.value = ''
  const targetEditor = action.method === 'config.update' ? editors[String(action.payload.id)] : undefined
  if (targetEditor) { targetEditor.error = ''; targetEditor.notice = '' }
  try {
    const payload = { ...action.payload, ...(action.grants ? { grants: action.chosen ?? [] } : {}), request_key: crypto.randomUUID() }
    const ack = await api.rpc<{ id: string; state: string }>(action.method, payload)
    if (!authenticated.value) return
    confirmation.value = null
    notice.value = `操作已受理，正在查询结果。编号：${ack.id}`
    if (targetEditor) { targetEditor.pending = ack.id; targetEditor.notice = '修改已提交，等待操作结果。编辑内容已保留，请勿重复保存。' }
    if (action.method === 'plugins.install') { showInstall.value = false; installPath.value = '' }
    await refresh(true)
  } catch (e) { if (e instanceof ApiError && e.code === 'StaleSession') return;
    const message = explain(e)
    if (targetEditor) targetEditor.error = message
    error.value = message
    confirmation.value = null
  } finally { busy.value = false }
}
async function diagnostics() {
  if (!authenticated.value || diagnosticBusy) return
  diagnosticBusy = true; diagnosticError.value = ''
  try {
    const data = await api.rpc<{ entries: LogEntry[] }>('diagnostics.logs')
    if (authenticated.value) logs.value = data.entries
  } catch (e) { if (e instanceof ApiError && e.code === 'StaleSession') return; diagnosticError.value = explain(e) } finally { diagnosticBusy = false }
}
async function checkAdapter() {
  if (!adapterId.value || busy.value) return
  busy.value = true; adapterError.value = ''; adapter.value = undefined
  try { const status = await api.rpc<Json>('adapter.status', { id: adapterId.value }); if (authenticated.value) adapter.value = status }
  catch (e) { if (e instanceof ApiError && e.code === 'StaleSession') return; adapterError.value = explain(e) } finally { busy.value = false }
}
function navigate(next: Page) { page.value = next; if (next === 'diagnostics') void diagnostics() }
onMounted(async () => {
  try { await api.session(); authenticated.value = true; error.value = ''; await refresh() }
  catch (e) { if (e instanceof ApiError && e.code === 'StaleSession') return; if (!(e instanceof ApiError && e.status === 401)) error.value = explain(e); else error.value = '' }
  finally { booting.value = false }
  timer = setInterval(() => { void refresh(); if (page.value === 'diagnostics') void diagnostics() }, 2500)
})
onUnmounted(() => { if (timer) clearInterval(timer) })
</script>

<template>
  <div v-if="booting" class="loading-screen" role="status">正在连接 TwinTail…</div>
  <main v-else-if="!authenticated" class="login-layout">
    <section class="login-story"><div class="brand"><span class="brand-mark">T</span> TwinTail<span class="brand-tag">CONTROL</span></div><div><p class="eyebrow">让每一个连接，井然有序</p><h1>你的插件。<br>你的节奏。</h1><p>在一个清晰的工作台里，管理运行状态、<br>调整配置，关注每一次连接。</p></div><div class="story-footer"><span class="live-dot"></span> 本地管理 · 独立插件 · 可控运行</div></section>
    <section class="login-panel"><div class="login-card"><p class="eyebrow accent">管理控制台</p><h2>欢迎回来</h2><p class="muted">使用本机管理凭据，进入你的工作台。</p><form @submit.prevent="login"><label for="token">管理令牌</label><input id="token" v-model="token" type="password" autocomplete="off" placeholder="输入管理令牌" required :disabled="busy" spellcheck="false"><p class="field-help">由本机管理员配置。令牌不会保存在浏览器中。</p><p v-if="error" role="alert" class="alert error">{{ error }}</p><button class="primary wide" :disabled="busy || !token.trim()">{{ busy ? '正在验证…' : '进入控制台 →' }}</button></form><div class="login-footnote">TwinTail 管理入口<br><span>请妥善保管管理凭据，仅在可信设备登录。</span></div></div></section>
  </main>
  <div v-else class="app-layout">
    <aside class="sidebar"><a href="#" class="brand" @click.prevent="navigate('overview')"><span class="brand-mark">T</span>TwinTail</a><p class="rail-label">工作空间</p><nav aria-label="主导航"><button :class="{ selected: page === 'overview' }" @click="navigate('overview')"><span>◫</span>运行概览</button><button :class="{ selected: page === 'plugins' }" @click="navigate('plugins')"><span>▦</span>插件管理 <small>{{ plugins.length }}</small></button><button :class="{ selected: page === 'operations' }" @click="navigate('operations')"><span>◷</span>操作记录 <small v-if="runningOps.length">{{ runningOps.length }}</small></button><button :class="{ selected: page === 'diagnostics' }" @click="navigate('diagnostics')"><span>≋</span>安全诊断</button></nav><div class="sidebar-bottom"><div class="workspace"><span class="workspace-icon">本</span><div>本机工作空间<small><span class="live-dot"></span> 会话已连接</small></div></div><button class="logout" :disabled="busy" @click="logout">退出登录 ↗</button></div></aside>
    <div class="main-shell"><header class="topbar"><span>TwinTail <span class="breadcrumb">/ 管理控制台</span></span><span class="connection"><span v-if="!writeAllowed" class="readonly-badge">只读模式</span><span class="live-dot"></span> {{ refreshing ? '正在同步' : '自动同步中' }}</span></header>
      <main class="content"><div v-if="error" class="alert error global-alert" role="alert"><span>{{ error }}</span><button aria-label="关闭错误提示" @click="error = ''">×</button></div><div v-if="notice" class="alert info global-alert" role="status"><span>{{ notice }}</span><button aria-label="关闭提示" @click="notice = ''">×</button></div>
        <template v-if="page === 'overview'"><div class="page-heading"><div><p class="eyebrow accent">工作空间 / OVERVIEW</p><h1>运行概览</h1><p class="muted">一眼了解插件与工作空间的运行情况。</p></div><button class="secondary" :disabled="refreshing" @click="refresh(true)">↻ 刷新状态</button></div>
          <section class="stats" aria-label="运行统计"><article><span>运行中的插件</span><strong>{{ activeCount }}<small>/ {{ plugins.length }}</small></strong><p><i class="live-dot"></i> 已启用插件的实时状态</p></article><article><span>已安装版本</span><strong>{{ packages.length.toString().padStart(2, '0') }}</strong><p>本地可用的插件包</p></article><article><span>正在执行</span><strong>{{ runningOps.length.toString().padStart(2, '0') }}</strong><p>正在处理的管理操作</p></article><article :class="{ attention: failedCount }"><span>需要关注</span><strong>{{ failedCount.toString().padStart(2, '0') }}</strong><p>{{ failedCount ? '请检查插件错误与诊断' : '当前没有插件报告异常' }}</p></article></section>
          <section class="panel"><div class="panel-heading"><div><h2>插件运行状态</h2><p>已启用插件与当前代次</p></div><button class="text-button" @click="navigate('plugins')">管理插件 →</button></div><div v-if="!plugins.length" class="empty"><span>▦</span><h3>从第一个插件开始</h3><p>安装可信的本地插件包，再为它授权并启用。</p><button class="primary" :disabled="!writeAllowed" @click="navigate('plugins'); showInstall = true">安装插件</button></div><div v-else class="plugin-grid"><button v-for="plugin in plugins" :key="plugin.id" class="plugin-card" @click="choose(plugin.id, plugin.version)"><div class="plugin-card-top"><span class="plugin-icon">{{ plugin.id.slice(0, 1).toUpperCase() }}</span><span class="badge" :data-state="plugin.state">{{ label(plugin.state) }}</span></div><h3>{{ plugin.id }}</h3><p>v{{ plugin.version }} <span>· 代次 {{ plugin.generation }}</span></p><p v-if="plugin.error" class="error-text">{{ plugin.error.message }}（{{ plugin.error.code }}）</p><div class="card-bottom">查看详情与配置 <span>↗</span></div></button></div></section>
          <section class="panel"><div class="panel-heading"><div><h2>最近操作</h2><p>每一次改变都有迹可循</p></div><button class="text-button" @click="navigate('operations')">全部记录 →</button></div><div v-if="!operations.length" class="empty compact">尚无操作记录</div><div v-for="op in recentOps.slice(0, 3)" :key="op.id" class="operation-row"><span class="operation-icon">↗</span><div><strong>{{ label(op.method) }}</strong><small>{{ op.target || '本地插件包' }} · {{ time(op.created_unix_ms) }}</small><p v-if="op.result?.code || op.result?.outcome" class="error-text">{{ label(op.result.outcome || op.result.code || '') }}</p></div><span class="badge" :data-state="op.state">{{ label(op.state) }}</span></div></section>
        </template>
        <template v-if="page === 'plugins'"><div class="page-heading"><div><p class="eyebrow accent">工作空间 / PLUGINS</p><h1>插件管理</h1><p class="muted">连接能力与工作流，让每个插件各司其职。</p></div><button class="primary" :disabled="!writeAllowed" @click="showInstall = !showInstall">＋ 安装本地插件</button></div>
          <section v-if="showInstall" class="panel install-panel"><h2>安装本地插件包</h2><p class="muted">输入宿主所在设备上的完整目录路径。安装不会自动授予权限。</p><form class="inline-form" @submit.prevent="askInstall"><label class="sr-only" for="install-path">插件包目录</label><input id="install-path" v-model="installPath" placeholder="例如 /opt/twintail/packages/my-plugin" required><button class="primary" :disabled="!writeAllowed || busy || !installPath.trim()">检查并安装</button></form></section>
          <section class="panel"><div class="list-toolbar"><div class="tabs" role="tablist" aria-label="插件列表类型"><button role="tab" :aria-selected="listMode === 'running'" :class="{ active: listMode === 'running' }" @click="listMode = 'running'">已启用 <span>{{ plugins.length }}</span></button><button role="tab" :aria-selected="listMode === 'installed'" :class="{ active: listMode === 'installed' }" @click="listMode = 'installed'">已安装 <span>{{ packages.length }}</span></button></div><input v-model="search" class="search" aria-label="搜索插件" placeholder="搜索插件名称或版本…"></div><div class="table-scroll"><table v-if="listMode === 'running'"><thead><tr><th>插件</th><th>版本</th><th>状态</th><th>代次</th><th class="align-right">管理</th></tr></thead><tbody><tr v-for="plugin in visiblePlugins" :key="plugin.id" :class="{ 'selected-row': selected === plugin.id }"><td><strong>{{ plugin.id }}</strong><p v-if="plugin.error" class="error-text">{{ plugin.error.message }}（{{ plugin.error.code }}）</p></td><td class="mono">{{ plugin.version }}</td><td><span class="badge" :data-state="plugin.state">{{ label(plugin.state) }}</span></td><td class="mono">{{ plugin.generation }}</td><td class="align-right"><button class="text-button" @click="choose(plugin.id, plugin.version)">详情 →</button></td></tr><tr v-if="!visiblePlugins.length"><td colspan="5" class="empty compact">没有匹配的已启用插件</td></tr></tbody></table><table v-else><thead><tr><th>插件</th><th>版本</th><th>启用状态</th><th class="align-right">管理</th></tr></thead><tbody><tr v-for="pkg in visiblePackages" :key="`${pkg.id}/${pkg.version}`"><td><strong>{{ pkg.id }}</strong></td><td class="mono">{{ pkg.version }}</td><td>{{ plugins.some(p => p.id === pkg.id && p.version === pkg.version) ? '当前启用' : '已安装' }}</td><td class="align-right"><button class="text-button" @click="choose(pkg.id, pkg.version)">详情 →</button></td></tr><tr v-if="!visiblePackages.length"><td colspan="4" class="empty compact">没有匹配的已安装版本</td></tr></tbody></table></div></section>
          <section v-if="selected" class="panel detail-panel"><div class="panel-heading"><div><p class="eyebrow accent">插件详情</p><h2>{{ selected }}</h2><p>v{{ selectedVersion }} <span v-if="current">· {{ label(current.state) }} · 代次 {{ current.generation }}</span></p></div><button class="text-button" @click="selected = ''">收起 ×</button></div><p v-if="current?.error" class="alert error" role="alert">{{ current.error.message }}（{{ current.error.code }}）</p><div class="detail-actions"><template v-if="current"><button class="secondary" :disabled="!writeAllowed || busy || current.state === 'active'" @click="askAction('plugins.start', selected)">启动</button><button class="secondary" :disabled="!writeAllowed || busy || current.state === 'stopped'" @click="askAction('plugins.stop', selected)">停止</button><label class="sr-only" for="reload-version">重载目标版本</label><select id="reload-version" v-model="selectedVersion"><option v-for="pkg in versions" :key="pkg.version" :value="pkg.version">v{{ pkg.version }}</option><option v-if="!versions.length" :value="current.version">v{{ current.version }}</option></select><button class="secondary" :disabled="!writeAllowed || busy" @click="askAction('plugins.reload', selected, selectedVersion)">重载此版本</button><button class="danger-quiet" :disabled="!writeAllowed || busy" @click="askAction('plugins.disable', selected)">停用插件</button></template><button v-else class="primary" :disabled="!writeAllowed || busy || !versions.find(p => p.version === selectedVersion)" @click="askEnable(versions.find(p => p.version === selectedVersion)!)">授权并启用</button><button class="danger-quiet" :disabled="!writeAllowed || busy || current?.version === selectedVersion" @click="askAction('plugins.uninstall', selected, selectedVersion)">卸载此版本</button></div><p v-if="selected === 'web-admin'" class="alert warning">此插件提供当前管理界面。停止或重载会中断连接；请保留本机 CLI 作为恢复入口。</p>
            <template v-if="current"><div class="config-title"><div><h3>插件配置 <span v-if="dirty" class="unsaved">未保存修改</span></h3><p class="muted">保存前校验格式，修改会基于当前修订提交。</p></div><button class="text-button" :disabled="configBusy" @click="reloadConfig">重新读取配置</button></div><template v-if="editor"><div class="revision-bar"><span>期望修订 <strong>{{ editor.data.status.desired_revision }}</strong></span><span>已应用修订 <strong>{{ editor.data.status.applied_revision ?? '未确认' }}</strong></span><span class="badge" :data-state="editor.data.status.applied_revision === editor.data.status.desired_revision ? 'active' : 'degraded'">{{ editor.data.status.applied_revision === editor.data.status.desired_revision ? '已同步' : '尚未同步' }}</span></div><p v-if="editor.data.status.applied_revision !== editor.data.status.desired_revision" class="alert warning">期望配置尚未在运行中确认。请检查操作结果，必要时重新启动插件。</p><div class="config-layout"><div><label :for="`config-${selected}`">JSON 配置 <span class="muted">· 编辑基于修订 {{ editor.data.snapshot.revision }}</span></label><textarea :id="`config-${selected}`" v-model="editor.text" class="json-editor" spellcheck="false" aria-label="JSON 配置编辑器"></textarea><p class="field-help">仅保留秘密引用，不在此填写明文秘密。脱敏占位符会被阻止提交。</p></div><aside class="schema-panel"><h4>配置说明</h4><p>插件提供的 JSON Schema，供核对字段与约束。</p><pre v-if="editor.schema">{{ pretty(editor.schema) }}</pre><p v-else class="muted">该插件未提供配置 Schema。</p></aside></div><p v-if="editor.error" class="alert error" role="alert">{{ editor.error }}</p><p v-if="editor.notice" class="alert info" role="status">{{ editor.notice }}</p><div class="config-footer"><span class="muted">配置版本 {{ editor.data.snapshot.config_version }}</span><div><button class="secondary" :disabled="configBusy || busy" @click="validateConfig">校验配置</button><button class="primary" :disabled="!writeAllowed || configBusy || busy || !dirty || !!editor.pending" @click="saveConfig">保存并应用</button></div></div></template><div v-else class="empty compact">{{ configBusy ? '正在读取配置…' : '配置尚未载入，请点击重新读取配置。' }}</div></template>
          </section>
        </template>
        <template v-if="page === 'operations'"><div class="page-heading"><div><p class="eyebrow accent">工作空间 / ACTIVITY</p><h1>操作记录</h1><p class="muted">持续跟进管理操作，确认每一次变更的结果。</p></div><button class="secondary" :disabled="refreshing" @click="refresh(true)">↻ 刷新记录</button></div><section class="panel"><div v-if="!operations.length" class="empty"><span>◷</span><h3>还没有操作记录</h3><p>安装、启停和配置更新的结果会显示在这里。</p></div><article v-for="op in recentOps" :key="op.id" class="operation-detail"><div class="operation-row"><span class="operation-icon">↗</span><div><strong>{{ label(op.method) }}</strong><small>{{ op.target || '本地插件包' }} · {{ time(op.created_unix_ms) }}</small></div><span class="badge" :data-state="op.state">{{ label(op.state) }}</span></div><div class="operation-meta"><span>进度：{{ label(op.stage) }}</span><code>{{ op.id }}</code></div><p v-if="op.result?.outcome" :class="['alert', op.result.outcome === 'reloaded' ? 'info' : 'warning']">{{ label(op.result.outcome) }}{{ op.result.outcome === 'rolled_back' ? '。新版本未成功启用，已恢复旧版本；请检查后再试。' : '' }}</p><p v-if="op.state === 'failed' || op.state === 'interrupted'" class="alert error" role="alert">{{ op.result?.code && /conflict/i.test(op.result.code) ? '配置修订冲突。编辑内容已保留，请读取最新修订后核对。' : `操作${label(op.state)}，请检查插件状态和诊断后再操作。` }} <span v-if="op.result?.code">（{{ op.result.code }}）</span></p><p v-if="op.result?.revision" class="muted">已保存修订 {{ op.result.revision }}；请在配置页核对已应用修订。</p></article></section></template>
        <template v-if="page === 'diagnostics'"><div class="page-heading"><div><p class="eyebrow accent">工作空间 / DIAGNOSTICS</p><h1>安全诊断</h1><p class="muted">定位连接与管理操作异常，保留必要的诊断信息。</p></div><button class="secondary" @click="diagnostics">↻ 刷新诊断</button></div><p v-if="diagnosticError" class="alert error" role="alert">{{ diagnosticError }}</p><p v-if="adapterError" class="alert error adapter-error" role="alert">{{ adapterError }}</p><section class="panel install-panel"><h2>适配器连接状态</h2><p class="muted">选择已启用的适配器，查询连接健康与安全状态摘要。</p><div class="inline-form"><label class="sr-only" for="adapter-id">适配器插件</label><select id="adapter-id" v-model="adapterId"><option value="" disabled>选择适配器插件</option><option v-for="plugin in plugins" :key="plugin.id" :value="plugin.id">{{ plugin.id }}</option></select><button class="primary" :disabled="busy || !adapterId" @click="checkAdapter">查询状态</button></div><pre v-if="adapter !== undefined" class="diagnostic-json">{{ pretty(adapter) }}</pre></section><section class="panel"><div class="panel-heading"><div><h2>安全诊断日志</h2><p>最近最多 128 条管理操作摘要。不包含插件原始 stderr 或秘密配置。</p></div></div><div v-if="!logs.length" class="empty compact">暂无管理诊断日志</div><article v-for="entry in [...logs].reverse()" :key="entry.operation_id" class="log-row"><span class="mono muted">{{ time(entry.timestamp_unix_ms) }}</span><span><strong>{{ label(entry.method) }}</strong><small>{{ entry.operation_id }}</small></span><span class="badge" :data-state="entry.state">{{ label(entry.state) }}</span><span>{{ label(entry.stage) }}</span><pre v-if="entry.result">{{ pretty(entry.result) }}</pre></article></section><div class="rescue-card"><h3>需要恢复管理入口？</h3><p>在宿主设备上使用 TwinTail CLI 检查插件状态，恢复配置或重新启动 web-admin。</p><code>twintail-host plugins list</code><code>twintail-host plugins start web-admin</code></div></template>
        <footer class="page-footer"><span>TwinTail / 让连接更有序</span><span>本地管理工作空间</span></footer>
      </main>
    </div>
    <div v-if="confirmation" class="modal-backdrop" @click.self="!busy && (confirmation = null)"><section class="modal" role="dialog" aria-modal="true" aria-labelledby="confirm-title" @keydown.esc="!busy && (confirmation = null)"><p class="eyebrow accent">确认操作</p><h2 id="confirm-title">{{ confirmation.title }}</h2><p class="muted">{{ confirmation.description }}</p><dl class="confirm-summary"><template v-for="(value, key) in confirmation.payload" :key="key"><template v-if="key !== 'values'"><dt>{{ key === 'id' ? '插件' : key === 'version' ? '版本' : key === 'path' ? '本地目录' : key === 'expected_revision' ? '基于修订' : key }}</dt><dd>{{ value }}</dd></template></template></dl><div v-if="confirmation.grants" class="grants"><h3>能力授权</h3><p v-if="!confirmation.grants.length" class="muted">该插件未申请任何能力。</p><label v-for="grant in confirmation.grants" :key="grant"><input v-model="confirmation.chosen" type="checkbox" :value="grant" :disabled="confirmation.required?.includes(grant)"><span>{{ grant }}</span><small v-if="confirmation.required?.includes(grant)">必需</small></label></div><div v-if="confirmation.self" class="alert warning"><strong>管理界面可能中断</strong><p>继续前请准备本机 CLI 救援。重载后会话可能失效；若界面未恢复，请执行：</p><code>twintail-host plugins start web-admin</code><p>配置错误时请先通过本机 CLI 恢复有效配置，再启动插件。</p></div><div class="modal-actions"><button class="secondary" :disabled="busy" @click="confirmation = null">取消</button><button class="primary" :disabled="busy" @click="confirmAction">{{ busy ? '正在提交…' : '确认操作' }}</button></div></section></div>
  </div>
</template>
