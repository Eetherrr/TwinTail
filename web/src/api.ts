export class ApiError extends Error {
  constructor(public code: string, message: string, public status = 0) { super(message) }
}
// RPC values stay opaque; the browser never resolves secret references.
export type Json = unknown
export interface Plugin { id: string; version: string; state: string; generation: number; error?: { code: string; message: string } | null; reload?: Json }
export interface Package { id: string; version: string; path: string; checksum: string }
export interface Operation { id: string; state: string; stage: string; method: string; target: string | null; created_unix_ms?: number; result?: { status: string; code?: string; outcome?: string; revision?: number; needs_recovery?: boolean } }
export interface Configuration { snapshot: { revision: number; config_version: number; values: Json }; status: { desired_revision: number; applied_revision: number | null } }
export interface Session { csrf_token: string; expires_in_secs: number }
export class Api {
  csrf = ''
  epoch = 0
  invalidate() { this.epoch++; this.csrf = '' }
  onExpired: () => void = () => {}
  async request<T>(path: string, method: string, body?: unknown): Promise<T> {
    const epoch = this.epoch
    const checkEpoch = () => { if (epoch !== this.epoch) throw new ApiError('StaleSession', '旧会话请求已忽略') }
    const controller = new AbortController()
    const timeout = setTimeout(() => controller.abort(), 20_000)
    try {
      const headers: Record<string, string> = {}
      if (body !== undefined) headers['Content-Type'] = 'application/json'
      if (this.csrf) headers['X-CSRF-Token'] = this.csrf
      const response = await fetch(path, { method, credentials: 'same-origin', mode: 'same-origin', headers, body: body === undefined ? undefined : JSON.stringify(body), signal: controller.signal })
      checkEpoch()
      if (!response.ok) {
        const data = await response.json().catch(() => null)
        checkEpoch()
        if (response.status === 401) { this.invalidate(); this.onExpired() }
        throw new ApiError(data?.error?.code ?? `HTTP_${response.status}`, data?.error?.message ?? '请求未完成，请稍后重试', response.status)
      }
      if (response.status === 204) return undefined as T
      const data = await response.json() as T
      checkEpoch()
      return data
    } catch (error) {
      if (!(error instanceof ApiError && error.status === 401)) checkEpoch()
      if (error instanceof ApiError) throw error
      throw new ApiError('Network', '连接中断或请求超时。操作可能已受理，请先查看操作记录。')
    } finally { clearTimeout(timeout) }
  }
  async session(token?: string) {
    if (token !== undefined) this.invalidate()
    const data = await this.request<Session>('/api/v1/session', token === undefined ? 'GET' : 'POST', token === undefined ? undefined : { token })
    if (!data.csrf_token) throw new ApiError('Protocol', '会话响应缺少安全凭据')
    this.csrf = data.csrf_token
    return data
  }
  async logout() { await this.request<void>('/api/v1/session', 'DELETE'); this.invalidate() }
  async rpc<T>(method: string, payload: unknown = {}): Promise<T> {
    const response = await this.request<{ result: T }>('/api/v1/manage', 'POST', { method, payload })
    return response.result
  }
}
export function assertSafeNumbers(value: unknown): void {
  if (typeof value === 'number' && (!Number.isFinite(value) || (Number.isInteger(value) && !Number.isSafeInteger(value)))) {
    throw new ApiError('UnsafeInteger', '配置或修订含浏览器无法精确表示的数值，请使用本机 CLI 编辑。输入已保留。')
  }
  if (Array.isArray(value)) value.forEach(assertSafeNumbers)
  else if (value !== null && typeof value === 'object') Object.values(value).forEach(assertSafeNumbers)
}
export function parseConfiguration(text: string): Json {
  let value: Json
  try { value = JSON.parse(text) as Json } catch { throw new ApiError('InvalidJSON', 'JSON 格式有误，请检查引号、逗号和括号。编辑内容已保留。') }
  if (value === null || Array.isArray(value) || typeof value !== 'object') throw new ApiError('InvalidJSON', '配置必须是 JSON 对象。编辑内容已保留。')
  assertSafeNumbers(value)
  const placeholder = (item: Json): boolean => typeof item === 'string'
    ? /^(?:\*{3,}|•{3,}|<redacted>|\[redacted\]|REDACTED|<hidden>)$/i.test(item.trim())
    : Array.isArray(item) ? item.some(placeholder) : item !== null && typeof item === 'object' && Object.values(item).some(placeholder)
  if (placeholder(value)) throw new ApiError('Redacted', '配置含脱敏占位符，已阻止保存。请使用原有秘密引用，勿填写明文秘密。')
  return value
}
export function explain(error: unknown): string {
  if (error instanceof ApiError) {
    if (error.status === 409 || /conflict/i.test(error.code)) return '配置修订或操作发生冲突。输入已保留，请重新读取最新配置，核对后再提交。'
    if (error.status === 401) return '会话已过期，请重新登录。'
    if (error.status === 403) return '安全校验或权限检查未通过，请重新登录并检查授权。'
    return `${error.message}（${error.code}）`
  }
  return error instanceof Error ? error.message : '操作失败，请查看诊断。'
}
export const labels: Record<string, string> = { active: '运行中', stopped: '已停止', failed: '失败', starting: '启动中', ready: '就绪', draining: '停止中', degraded: '异常', disabled: '未启用', running: '执行中', completed: '已完成', interrupted: '已中断', accepted: '已受理', validating: '校验中', installing: '安装中', stopping: '停止中', preparing: '准备配置', persisting_config: '保存配置', committing: '应用配置', persisting_plan: '保存启用设置', reloaded: '重载完成', rolled_back: '已回退旧版本', 'plugins.start': '启动插件', 'plugins.stop': '停止插件', 'plugins.reload': '重载插件', 'plugins.enable': '授权并启用', 'plugins.disable': '停用插件', 'plugins.install': '安装本地插件', 'plugins.uninstall': '卸载版本', 'config.update': '更新配置' }
export const label = (value: string) => labels[value] ?? value
