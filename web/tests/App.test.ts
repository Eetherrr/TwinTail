import { mount, flushPromises, type VueWrapper } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import App from '../src/App.vue'
import type { Operation } from '../src/api'
const plugin = { id: 'echo-plugin', version: '1.0.0', state: 'active', generation: 1, error: null }
const snapshot = { snapshot: { revision: 4, config_version: 1, values: { prefix: 'hello', auth: { $secret: 'env:TEST_TOKEN' } } }, status: { desired_revision: 4, applied_revision: 4 }, schema: { type: 'object', properties: { prefix: { type: 'string' } } } }
const wrappers: VueWrapper[] = []
let pluginEnabled = true, fixtureId = plugin.id
let readonly = false, expired = false, loginRequired = false, conflict = false
let ops: Operation[] = [], calls: { method: string; payload: Record<string, unknown> }[] = []
const response = (value: unknown, status = 200) => new Response(JSON.stringify(value), { status })
let readmeReply: (payload: Record<string, unknown>) => Response | Promise<Response>
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(done => { resolve = done }); return { promise, resolve } }
function button(wrapper: VueWrapper, text: string) { const found = wrapper.findAll('button').find(b => b.text() === text); if (!found) throw new Error(`Missing button: ${text}`); return found }
async function openEditor(wrapper: VueWrapper) { await (wrapper.find('.plugin-card').exists() ? wrapper.get('.plugin-card') : wrapper.get('tbody tr button')).trigger('click'); await flushPromises(); return wrapper.get('textarea') }
async function setup() { const wrapper = mount(App); wrappers.push(wrapper); await flushPromises(); return wrapper }
beforeEach(() => {
  vi.stubGlobal('localStorage', { setItem: vi.fn(), getItem: vi.fn() }); vi.stubGlobal('sessionStorage', { setItem: vi.fn(), getItem: vi.fn() })
  vi.useFakeTimers(); pluginEnabled = true; fixtureId = plugin.id; readonly = false; expired = false; loginRequired = false; conflict = false; ops = []; calls = []
  readmeReply = () => response({ result: { filename: null, content: null, verified: false } })
  vi.stubGlobal('fetch', vi.fn(async (path: string, init: RequestInit) => {
    if (path === '/api/v1/session') {
      if (init.method === 'DELETE') return new Response(null, { status: 204 })
      if (init.method === 'GET' && loginRequired) return response({ error: { code: 'Unauthorized', message: 'expired' } }, 401)
      return response({ csrf_token: 'csrf-fixture', expires_in_secs: 600 })
    }
    if (expired) return response({ error: { code: 'Unauthorized', message: 'expired' } }, 401)
    const body = JSON.parse(String(init.body)); calls.push(body)
    if (body.method === 'plugins.readme') return readmeReply(body.payload)
    if (body.method === 'plugins.list') return response({ result: pluginEnabled ? [{ ...plugin, id: fixtureId }] : [] })
    if (body.method === 'plugins.installed') return response({ result: [{ id: fixtureId, version: plugin.version, path: '/fixture', checksum: 'x' }, { id: 'other-plugin', version: '2.0.0', path: '/other', checksum: 'y' }, { id: fixtureId, version: '1.1.0', path: '/fixture-new', checksum: 'z' }] })
    if (body.method === 'operations.list') return response({ result: ops })
    if (body.method === 'management.overview') return response({ result: { permissions: { read: true, write: !readonly }, readonly } })
    if (body.method === 'config.schema' || body.method === 'config.get') return response({ result: snapshot })
    if (body.method === 'config.validate') return response({ result: { valid: true } })
    if (body.method === 'plugins.inspect') return response({ result: { manifest: { capabilities: ['events.subscribe', 'actions.call'], required_capabilities: ['events.subscribe'] } } })
    if (body.method === 'config.update' && conflict) return response({ error: { code: 'Conflict', message: 'management request conflicts' } }, 409)
    if (body.method === 'adapter.status') return response({ error: { code: 'Unsupported', message: 'unknown mock method' } }, 400)
    if (body.method === 'diagnostics.logs') return response({ result: { entries: [] } })
    return response({ result: { id: 'op-test', state: 'running' } })
  }))
})
afterEach(() => { wrappers.splice(0).forEach(w => w.unmount()); vi.useRealTimers(); vi.unstubAllGlobals() })
describe('installed README viewer', () => {
  it('reads only on explicit request, uses selected identity, and keeps malicious HTML as text', async () => {
    readonly = true
    const malicious = '<script>alert(1)</script><img src="https://evil.test/track" onerror="alert(2)"><iframe src="https://evil.test"></iframe>\n# README'
    const filename = '<img src=x>.md'
    readmeReply = () => response({ result: { filename, content: malicious, verified: false } })
    const wrapper = await setup(); await openEditor(wrapper)
    expect(calls.filter(c => c.method === 'plugins.readme')).toHaveLength(0)
    await vi.advanceTimersByTimeAsync(5000); await flushPromises()
    expect(calls.filter(c => c.method === 'plugins.readme')).toHaveLength(0)
    await button(wrapper, '查看 README').trigger('click'); await flushPromises()
    expect(calls.find(c => c.method === 'plugins.readme')?.payload).toEqual({ id: plugin.id, version: '1.0.0' })
    const panel = wrapper.get('.readme-panel')
    expect(panel.text()).toContain(`${plugin.id} · v1.0.0`)
    expect(panel.get('.readme-filename').text()).toBe(filename)
    expect(panel.get('pre').text()).toBe(malicious)
    expect(panel.findAll('script,img,iframe,a')).toHaveLength(0)
    expect(panel.html()).toContain('&lt;script&gt;')
    expect(panel.text()).toContain('未验证')
    await vi.advanceTimersByTimeAsync(5000); await flushPromises()
    expect(calls.filter(c => c.method === 'plugins.readme')).toHaveLength(1)
  })
  it('shows missing and empty documents and reads installed but disabled plugins', async () => {
    const wrapper = await setup(); await openEditor(wrapper)
    await wrapper.findAll('[role="tab"]')[1].trigger('click')
    await wrapper.findAll('tbody tr').find(row => row.text().includes('other-plugin'))!.get('button').trigger('click')
    await button(wrapper, '查看 README').trigger('click'); await flushPromises()
    expect(calls.find(c => c.method === 'plugins.readme')?.payload).toEqual({ id: 'other-plugin', version: '2.0.0' })
    expect(wrapper.get('.readme-panel').text()).toContain('未提供 README')
    expect(wrapper.find('.readme-content').exists()).toBe(false)
    readmeReply = () => response({ result: { filename: 'README.txt', content: '', verified: false } })
    await button(wrapper, '查看 README').trigger('click'); await flushPromises()
    expect(wrapper.get('.readme-panel').text()).toContain('README 内容为空')
    expect(wrapper.get('.readme-filename').text()).toBe('README.txt')
  })
  it('shows loading and errors inline and allows retry', async () => {
    const pending = deferred<Response>()
    readmeReply = () => pending.promise
    const wrapper = await setup(); await openEditor(wrapper)
    await button(wrapper, '查看 README').trigger('click')
    expect(wrapper.get('.readme-panel [role="status"]').text()).toContain('正在读取')
    expect(button(wrapper, '查看 README').attributes('disabled')).toBeDefined()
    pending.resolve(response({ error: { code: 'Io', message: 'README read failed' } }, 400)); await flushPromises()
    expect(wrapper.get('.readme-panel [role="alert"]').text()).toContain('README read failed（Io）')
    expect(button(wrapper, '查看 README').attributes('disabled')).toBeUndefined()
    readmeReply = () => response({ result: { filename: 'README.md', content: 'retry document', verified: false } })
    await button(wrapper, '查看 README').trigger('click'); await flushPromises()
    expect(wrapper.get('.readme-content').text()).toBe('retry document')
    expect(wrapper.find('.readme-panel [role="alert"]').exists()).toBe(false)
  })
  it('clears old text on version change and ignores stale responses without clearing current loading', async () => {
    readmeReply = () => response({ result: { filename: 'README.md', content: 'old version', verified: false } })
    const wrapper = await setup(); await openEditor(wrapper)
    await button(wrapper, '查看 README').trigger('click'); await flushPromises()
    expect(wrapper.get('.readme-content').text()).toBe('old version')
    await wrapper.get('#reload-version').setValue('1.1.0')
    expect(wrapper.find('.readme-content').exists()).toBe(false)
    expect(calls.filter(c => c.method === 'plugins.readme')).toHaveLength(1)
    const stale = deferred<Response>(), latest = deferred<Response>()
    readmeReply = payload => payload.version === '1.1.0' ? stale.promise : latest.promise
    await button(wrapper, '查看 README').trigger('click')
    await wrapper.get('#reload-version').setValue('1.0.0')
    await button(wrapper, '查看 README').trigger('click')
    stale.resolve(response({ result: { filename: 'README.md', content: 'stale version', verified: false } })); await flushPromises()
    expect(wrapper.find('.readme-content').exists()).toBe(false)
    expect(wrapper.get('.readme-panel [role="status"]').text()).toContain('正在读取')
    latest.resolve(response({ result: { filename: 'README.md', content: 'current version', verified: false } })); await flushPromises()
    expect(wrapper.get('.readme-content').text()).toBe('current version')
  })
  it('ignores stale failures after selecting another plugin', async () => {
    const pending = deferred<Response>(); readmeReply = () => pending.promise
    const wrapper = await setup(); await openEditor(wrapper)
    await button(wrapper, '查看 README').trigger('click')
    await wrapper.findAll('[role="tab"]')[1].trigger('click')
    await wrapper.findAll('tbody tr').find(row => row.text().includes('other-plugin'))!.get('button').trigger('click')
    pending.resolve(response({ error: { code: 'Io', message: 'stale error' } }, 400)); await flushPromises()
    expect(wrapper.get('.readme-panel').text()).not.toContain('stale error')
    expect(wrapper.find('.readme-panel [role="status"]').exists()).toBe(false)
  })
  it('clears loaded README on collapse and discards pending reads across logout and login', async () => {
    readmeReply = () => response({ result: { filename: 'README.md', content: 'private document', verified: false } })
    const wrapper = await setup(); await openEditor(wrapper)
    await button(wrapper, '查看 README').trigger('click'); await flushPromises()
    await button(wrapper, '收起 ×').trigger('click'); await openEditor(wrapper)
    expect(wrapper.find('.readme-content').exists()).toBe(false)
    const pending = deferred<Response>(); readmeReply = () => pending.promise
    await button(wrapper, '查看 README').trigger('click')
    await button(wrapper, '退出登录 ↗').trigger('click'); await flushPromises()
    expect(wrapper.find('#token').exists()).toBe(true)
    await wrapper.get('#token').setValue('new-session-token')
    await wrapper.get('form').trigger('submit'); await flushPromises(); await openEditor(wrapper)
    pending.resolve(response({ result: { filename: 'README.md', content: 'previous session document', verified: false } })); await flushPromises()
    expect(wrapper.find('.readme-content').exists()).toBe(false)
    expect(wrapper.text()).not.toContain('previous session document')
    expect(wrapper.text()).not.toContain('private document')
    expect(button(wrapper, '查看 README').attributes('disabled')).toBeUndefined()
  })
})
describe('management interactions', () => {
  it('logs in without storing or rendering the token', async () => {
    loginRequired = true
    const wrapper = await setup()
    const token = 'x'.repeat(40)
    await wrapper.get('#token').setValue(token)
    await wrapper.get('form').trigger('submit'); await flushPromises()
    expect(wrapper.find('.app-layout').exists()).toBe(true)
    expect(localStorage.setItem).not.toHaveBeenCalled(); expect(sessionStorage.setItem).not.toHaveBeenCalled()
    expect(wrapper.html()).not.toContain(token)
  })
  it('sends the base revision and preserves draft through a direct conflict', async () => {
    const wrapper = await setup(), area = await openEditor(wrapper)
    const draft = '{"prefix":"edited","auth":{"$secret":"env:TEST_TOKEN"}}'
    await area.setValue(draft); conflict = true
    await button(wrapper, '保存并应用').trigger('click'); await flushPromises()
    expect(wrapper.get('[role="dialog"]').text()).toContain('修订 4')
    await button(wrapper, '确认操作').trigger('click'); await flushPromises()
    expect(calls.find(c => c.method === 'config.update')?.payload).toMatchObject({ expected_revision: 4, values: JSON.parse(draft) })
    expect((area.element as HTMLTextAreaElement).value).toBe(draft)
    expect(wrapper.text()).toContain('输入已保留')
    expect(wrapper.text()).toContain('编辑基于修订 4')
  })
  it('polls async failure and keeps both text and base revision while showing Conflict inline', async () => {
    const wrapper = await setup(), area = await openEditor(wrapper)
    await area.setValue('{"prefix":"kept"}')
    await button(wrapper, '保存并应用').trigger('click'); await button(wrapper, '确认操作').trigger('click'); await flushPromises()
    ops = [{ id: 'op-test', method: 'config.update', target: plugin.id, state: 'failed', stage: 'failed', result: { status: 'error', code: 'conflict' } }]
    await vi.advanceTimersByTimeAsync(2500); await flushPromises()
    expect((area.element as HTMLTextAreaElement).value).toBe('{"prefix":"kept"}')
    expect(wrapper.text()).toContain('配置修订冲突。编辑内容和基准修订已保留')
    expect(wrapper.text()).toContain('编辑基于修订 4')
  })
  it('blocks malformed JSON and redacted placeholders before any HTTP mutation', async () => {
    const wrapper = await setup(), area = await openEditor(wrapper)
    await area.setValue('{"token":"***"}'); await button(wrapper, '保存并应用').trigger('click')
    expect(wrapper.text()).toContain('脱敏占位符'); expect(calls.some(c => c.method === 'config.update')).toBe(false)
    await area.setValue('{'); await button(wrapper, '校验配置').trigger('click')
    expect(wrapper.text()).toContain('JSON 格式有误'); expect(calls.some(c => c.method === 'config.validate')).toBe(false)
  })
  it('requires explicit replacement of a dirty draft', async () => {
    const wrapper = await setup(), area = await openEditor(wrapper)
    await area.setValue('{"prefix":"unsaved"}'); await button(wrapper, '重新读取配置').trigger('click')
    expect(wrapper.get('[role="dialog"]').text()).toContain('替换当前编辑内容')
    await button(wrapper, '取消').trigger('click')
    expect((area.element as HTMLTextAreaElement).value).toContain('unsaved')
  })
  it('shows readonly permissions and disables every mutation while leaving validation available', async () => {
    readonly = true
    const wrapper = await setup()
    expect(wrapper.text()).toContain('只读模式')
    await openEditor(wrapper)
    for (const text of ['＋ 安装本地插件', '启动', '停止', '重载此版本', '停用插件', '卸载此版本', '保存并应用']) expect(button(wrapper, text).attributes('disabled')).toBeDefined()
    expect(button(wrapper, '校验配置').attributes('disabled')).toBeUndefined()
  })
  it('requires confirmation and prevents double submission of privileged operations', async () => {
    const wrapper = await setup(); await openEditor(wrapper)
    await button(wrapper, '停止').trigger('click')
    expect(calls.some(c => c.method === 'plugins.stop')).toBe(false)
    const confirm = button(wrapper, '确认操作')
    await Promise.all([confirm.trigger('click'), confirm.trigger('click')]); await flushPromises()
    expect(calls.filter(c => c.method === 'plugins.stop')).toHaveLength(1)
    expect(calls.find(c => c.method === 'plugins.stop')?.payload.request_key).toBeTypeOf('string')
  })
  it('expires to the login page and removes loaded configuration', async () => {
    const wrapper = await setup(); const area = await openEditor(wrapper)
    await area.setValue('{"prefix":"sensitive-draft"}'); expired = true
    await vi.advanceTimersByTimeAsync(2500); await flushPromises()
    expect(wrapper.find('#token').exists()).toBe(true)
    expect(wrapper.text()).toContain('会话已过期')
    expect(wrapper.html()).not.toContain('sensitive-draft')
  })
  it('polls runtime state without repeatedly rescanning installed packages', async () => {
    const wrapper = await setup()
    expect(calls.filter(c => c.method === 'plugins.installed')).toHaveLength(1)
    await vi.advanceTimersByTimeAsync(5000); await flushPromises()
    expect(calls.filter(c => c.method === 'plugins.list').length).toBeGreaterThan(1)
    expect(calls.filter(c => c.method === 'plugins.installed')).toHaveLength(1)
    await button(wrapper, '↻ 刷新状态').trigger('click'); await flushPromises()
    expect(calls.filter(c => c.method === 'plugins.installed')).toHaveLength(2)
  })
  it('keeps adapter errors visible when diagnostic logs refresh', async () => {
    const wrapper = await setup()
    await wrapper.get('nav').findAll('button')[3].trigger('click'); await flushPromises()
    await wrapper.get('#adapter-id').setValue('echo-plugin')
    await button(wrapper, '查询状态').trigger('click'); await flushPromises()
    expect(wrapper.get('.adapter-error').text()).toContain('Unsupported')
    await vi.advanceTimersByTimeAsync(2500); await flushPromises()
    expect(wrapper.get('.adapter-error').text()).toContain('Unsupported')
  })
  it('uses manifest capability checkboxes for explicit enable grants', async () => {
    const wrapper = await setup()
    await wrapper.get('.plugin-card').trigger('click'); await flushPromises()
    await wrapper.findAll('[role="tab"]')[1].trigger('click')
    const otherRow = wrapper.findAll('tbody tr').find(row => row.text().includes('other-plugin'))!
    await otherRow.get('button').trigger('click'); await flushPromises()
    await button(wrapper, '授权并启用').trigger('click'); await flushPromises()
    const checkboxes = wrapper.findAll('.grants input')
    expect(checkboxes[0].attributes('disabled')).toBeDefined()
    await checkboxes[1].setValue(true)
    await button(wrapper, '确认操作').trigger('click'); await flushPromises()
    expect(calls.find(c => c.method === 'plugins.enable')?.payload.grants).toEqual(['events.subscribe', 'actions.call'])
  })
})

describe('independent destructive plugin purge', () => {
  async function openDisabled(wrapper: VueWrapper) {
    await wrapper.findAll('nav button')[1].trigger('click')
    await wrapper.findAll('[role="tab"]')[1].trigger('click')
    await wrapper.findAll('tbody tr').find(row => row.text().includes(fixtureId))!.get('button').trigger('click')
    await flushPromises()
  }
  it('requires disable and keeps version uninstall separate', async () => {
    const wrapper = await setup(); await openEditor(wrapper)
    expect(button(wrapper, '彻底移除插件').attributes('disabled')).toBeDefined()
    expect(wrapper.get('.purge-panel').text()).toContain('请先停用插件')
    expect(wrapper.get('.purge-panel').text()).toContain('仅停止进程')
    expect(button(wrapper, '卸载此版本').exists()).toBe(true)
    expect(calls.some(c => c.method === 'plugins.purge')).toBe(false)
  })
  it('requires exact ID, previews all versions and follows operation ACK and failure stages', async () => {
    pluginEnabled = false
    const wrapper = await setup(); await openDisabled(wrapper)
    await button(wrapper, '彻底移除插件').trigger('click')
    const dialog = wrapper.get('[role="dialog"]')
    expect(dialog.text()).toContain('1.0.0'); expect(dialog.text()).toContain('1.1.0')
    expect(dialog.text()).toContain('配置和业务数据'); expect(dialog.text()).toContain('不可恢复')
    expect(button(wrapper, '确认操作').attributes('disabled')).toBeDefined()
    for (const id of ['ECHO-PLUGIN', ' echo-plugin', 'echo-plugin ', 'other-plugin']) {
      await wrapper.get('#purge-confirm-id').setValue(id)
      expect(button(wrapper, '确认操作').attributes('disabled')).toBeDefined()
    }
    await wrapper.get('#purge-confirm-id').setValue('echo-plugin')
    expect(button(wrapper, '确认操作').attributes('disabled')).toBeUndefined()
    await button(wrapper, '确认操作').trigger('click'); await flushPromises()
    expect(calls.find(c => c.method === 'plugins.purge')!.payload).toEqual({ id: 'echo-plugin', confirm_id: 'echo-plugin', request_key: expect.any(String) })
    expect(wrapper.text()).toContain('操作已受理')
    ops = [{ id: 'op-test', method: 'plugins.purge', state: 'failed', stage: 'purging_data', target: 'echo-plugin', result: { status: 'err', code: 'io' } }]
    await vi.advanceTimersByTimeAsync(2500); await flushPromises()
    await wrapper.findAll('nav button')[2].trigger('click')
    expect(wrapper.text()).toContain('删除配置与数据'); expect(wrapper.text()).toContain('io')
    await wrapper.findAll('nav button')[1].trigger('click')
    await button(wrapper, '彻底移除插件').trigger('click')
    expect((wrapper.get('#purge-confirm-id').element as HTMLInputElement).value).toBe('')
    expect(button(wrapper, '确认操作').attributes('disabled')).toBeDefined()
  })
  it('blocks a stale confirmation when enabled elsewhere', async () => {
    pluginEnabled = false
    const wrapper = await setup(); await openDisabled(wrapper)
    await button(wrapper, '彻底移除插件').trigger('click'); await wrapper.get('#purge-confirm-id').setValue(fixtureId)
    pluginEnabled = true
    await vi.advanceTimersByTimeAsync(2500); await flushPromises()
    expect(button(wrapper, '确认操作').attributes('disabled')).toBeDefined()
    expect(wrapper.get('[role="dialog"]').text()).toContain('请取消并先停用')
    expect(calls.some(c => c.method === 'plugins.purge')).toBe(false)
  })
  it('cancels without submission, enforces read-only and warns about CLI reinstall for self-removal', async () => {
    pluginEnabled = false; fixtureId = 'web-admin'
    const wrapper = await setup(); await openDisabled(wrapper)
    await button(wrapper, '彻底移除插件').trigger('click')
    expect(wrapper.get('[role="dialog"]').text()).toContain('必须通过本机 CLI 重新安装包')
    expect(wrapper.get('[role="dialog"]').text()).toContain('恢复有效配置并授权启用')
    await button(wrapper, '取消').trigger('click')
    expect(calls.some(c => c.method === 'plugins.purge')).toBe(false)
    readonly = true; await vi.advanceTimersByTimeAsync(2500); await flushPromises()
    expect(button(wrapper, '彻底移除插件').attributes('disabled')).toBeDefined()
  })
})
